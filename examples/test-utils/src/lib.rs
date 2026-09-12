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
    db_name: String,
    pub pool: sqlx::PgPool,
}

impl AsyncTestContext for SqlxPgContext {
    async fn setup() -> Self {
        let database_url = postgres_url();
        let admin_pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        let db_name = generate_tmp_db();
        sqlx::query(&format!("CREATE DATABASE {db_name}"))
            .execute(&admin_pool)
            .await
            .unwrap();
        admin_pool.close().await;

        let mut test_url = url::Url::parse(&database_url).unwrap();
        test_url.set_path(&format!("/{db_name}"));
        let pool = sqlx::PgPool::connect(test_url.as_str()).await.unwrap();
        Self { db_name, pool }
    }

    async fn teardown(self) {
        self.pool.close().await;
        let admin_pool = sqlx::PgPool::connect(&postgres_url()).await.unwrap();
        sqlx::query(&format!("DROP DATABASE {}", self.db_name))
            .execute(&admin_pool)
            .await
            .unwrap();
        admin_pool.close().await;
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
