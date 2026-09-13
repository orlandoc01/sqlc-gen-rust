use std::str::FromStr as _;

use test_context::{AsyncTestContext, TestContext};

fn generate_tmp_db() -> String {
    let suffix = std::iter::repeat_with(fastrand::alphanumeric)
        .take(10)
        .collect::<String>();
    format!("test_db_{suffix}").to_lowercase()
}

fn postgres_url() -> String {
    std::env::var("POSTGRES_DATABASE_URL")
        .or(std::env::var("DATABASE_URL"))
        .unwrap()
}

pub struct SqlxPgContext {
    database: PgDatabase,
    pub pool: sqlx::PgPool,
}

struct PgDatabase {
    admin_url: url::Url,
    db_name: String,
}

impl PgDatabase {
    fn setup() -> Self {
        let database_url = postgres_url();
        let mut admin = postgres::Client::connect(&database_url, postgres::NoTls).unwrap();
        let db_name = generate_tmp_db();
        admin
            .batch_execute(&format!("CREATE DATABASE {db_name}"))
            .unwrap();
        Self {
            admin_url: url::Url::parse(&database_url).unwrap(),
            db_name,
        }
    }

    fn test_url(&self) -> url::Url {
        let mut test_url = self.admin_url.clone();
        test_url.set_path(&format!("/{}", self.db_name));
        test_url
    }

    fn teardown(self) {
        let mut admin =
            postgres::Client::connect(self.admin_url.as_str(), postgres::NoTls).unwrap();
        // FORCE: a closed pool's connections may still be winding down server-side.
        admin
            .batch_execute(&format!("DROP DATABASE {} WITH (FORCE)", self.db_name))
            .unwrap();
    }
}

impl AsyncTestContext for SqlxPgContext {
    async fn setup() -> Self {
        let database = tokio::task::spawn_blocking(PgDatabase::setup)
            .await
            .unwrap();
        let pool = sqlx::PgPool::connect(database.test_url().as_str())
            .await
            .unwrap();
        Self { database, pool }
    }

    async fn teardown(self) {
        let Self { database, pool } = self;
        pool.close().await;
        tokio::task::spawn_blocking(move || database.teardown())
            .await
            .unwrap();
    }
}

pub struct PgTokioContext {
    database: PgDatabase,
    pub client: tokio_postgres::Client,
    connection_task: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
}

impl AsyncTestContext for PgTokioContext {
    async fn setup() -> Self {
        let database = tokio::task::spawn_blocking(PgDatabase::setup)
            .await
            .unwrap();
        let (client, connection) =
            tokio_postgres::connect(database.test_url().as_str(), tokio_postgres::NoTls)
                .await
                .unwrap();
        Self {
            database,
            client,
            connection_task: tokio::spawn(connection),
        }
    }

    async fn teardown(self) {
        let Self {
            database,
            client,
            connection_task,
        } = self;
        drop(client);
        connection_task.await.unwrap().unwrap();
        tokio::task::spawn_blocking(move || database.teardown())
            .await
            .unwrap();
    }
}

pub struct PgDeadpoolContext {
    database: PgDatabase,
    pub pool: deadpool_postgres::Pool,
}

impl AsyncTestContext for PgDeadpoolContext {
    async fn setup() -> Self {
        let database = tokio::task::spawn_blocking(PgDatabase::setup)
            .await
            .unwrap();
        let config = tokio_postgres::Config::from_str(database.test_url().as_str()).unwrap();
        let manager = deadpool_postgres::Manager::from_config(
            config,
            tokio_postgres::NoTls,
            deadpool_postgres::ManagerConfig {
                recycling_method: deadpool_postgres::RecyclingMethod::Fast,
            },
        );
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .unwrap();
        Self { database, pool }
    }

    async fn teardown(self) {
        let Self { database, pool } = self;
        pool.close();
        tokio::task::spawn_blocking(move || database.teardown())
            .await
            .unwrap();
    }
}

pub struct PgSyncContext {
    database: PgDatabase,
    pub client: postgres::Client,
}

impl TestContext for PgSyncContext {
    fn setup() -> Self {
        let database = PgDatabase::setup();
        let client =
            postgres::Client::connect(database.test_url().as_str(), postgres::NoTls).unwrap();
        Self { database, client }
    }

    fn teardown(self) {
        let Self { database, client } = self;
        drop(client);
        database.teardown();
    }
}

pub struct SqlxMysqlContext {
    db_name: String,
    pub pool: sqlx::MySqlPool,
}

fn mysql_url() -> String {
    std::env::var("MYSQL_DATABASE_URL")
        .or(std::env::var("DATABASE_URL"))
        .unwrap()
}

fn mysql_config() -> (u16, String, String, String) {
    let database_url = mysql_url();
    let mysql_url = url::Url::parse(&database_url).unwrap();
    let host = mysql_url
        .host()
        .map(|host| host.to_string())
        .unwrap_or_else(|| "localhost".into());
    let port = mysql_url.port().unwrap_or(3306);
    let user = mysql_url.username().to_string();
    let password = mysql_url.password().unwrap_or("").to_string();
    (port, host, user, password)
}

impl AsyncTestContext for SqlxMysqlContext {
    async fn setup() -> Self {
        let (port, host, user, password) = mysql_config();
        let admin_url = format!("mysql://{user}:{password}@{host}:{port}/mysql");
        let admin_pool = sqlx::MySqlPool::connect(&admin_url).await.unwrap();
        let db_name = generate_tmp_db();
        sqlx::query(&format!("CREATE DATABASE `{db_name}`"))
            .execute(&admin_pool)
            .await
            .unwrap();
        admin_pool.close().await;

        let test_url = format!("mysql://{user}:{password}@{host}:{port}/{db_name}");
        let pool = sqlx::MySqlPool::connect(&test_url).await.unwrap();
        Self { db_name, pool }
    }

    async fn teardown(self) {
        self.pool.close().await;
        let (port, host, user, password) = mysql_config();
        let admin_url = format!("mysql://{user}:{password}@{host}:{port}/mysql");
        let admin_pool = sqlx::MySqlPool::connect(&admin_url).await.unwrap();
        sqlx::query(&format!("DROP DATABASE `{}`", self.db_name))
            .execute(&admin_pool)
            .await
            .unwrap();
        admin_pool.close().await;
    }
}

pub struct SqlxSqliteContext {
    pub pool: sqlx::SqlitePool,
}

impl AsyncTestContext for SqlxSqliteContext {
    async fn setup() -> Self {
        Self {
            pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
        }
    }
}

pub struct RusqliteContext {
    pub conn: rusqlite::Connection,
}

impl TestContext for RusqliteContext {
    fn setup() -> Self {
        Self {
            conn: rusqlite::Connection::open_in_memory().unwrap(),
        }
    }
}
