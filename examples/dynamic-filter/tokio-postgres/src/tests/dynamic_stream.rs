use futures_util::TryStreamExt as _;
use test_context::test_context;
use test_utils::PgTokioContext;

use super::queries;

async fn migrate(client: &tokio_postgres::Client) {
    client
        .batch_execute(include_str!("../../schema.sql"))
        .await
        .unwrap();
    client
        .batch_execute(
            r#"INSERT INTO users (id, email, phone, profile) VALUES
                (1, 'alice@example.com', '111', '{"tier":"gold"}'),
                (2, 'bob@example.com', '222', '{"tier":"silver"}'),
                (3, 'carol@example.com', '333', '{"tier":"gold"}');"#,
        )
        .await
        .unwrap();
}

fn params() -> queries::SearchUsersParams<'static> {
    queries::SearchUsersParams {
        row_limit: 100,
        ..Default::default()
    }
}

#[test_context(PgTokioContext)]
#[tokio::test]
async fn drains_dynamic_streams_and_detaches_owned_params(ctx: &mut PgTokioContext) {
    let client = &ctx.client;
    migrate(client).await;

    let stream = queries::search_users_stream(client, params())
        .await
        .unwrap();
    futures_util::pin_mut!(stream);
    let users = stream.try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(
        users
            .iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );

    let empty = [];
    let stream = queries::search_users_stream(
        client,
        queries::SearchUsersParams {
            email: Some("alice@example.com"),
            ids: Some(&empty),
            ..params()
        },
    )
    .await
    .unwrap();
    futures_util::pin_mut!(stream);
    assert!(stream.try_collect::<Vec<_>>().await.unwrap().is_empty());

    let ids = [1, 3];
    let stream = queries::search_users_stream(
        client,
        queries::SearchUsersParams {
            email: Some("alice@example.com"),
            ids: Some(&ids),
            ..params()
        },
    )
    .await
    .unwrap();
    futures_util::pin_mut!(stream);
    let users = stream.try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(
        users
            .iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1]
    );

    let stream = {
        let profile = serde_json::json!({"tier": "gold"});
        queries::search_users_by_profile_stream(
            client,
            queries::SearchUsersByProfileParams {
                email: None,
                profile: Some(profile),
            },
        )
        .await
        .unwrap()
    };
    futures_util::pin_mut!(stream);
    let users = stream.try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(
        users
            .iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1, 3]
    );

    assert_eq!(
        queries::search_users_by_profile(
            client,
            queries::SearchUsersByProfileParams {
                email: None,
                profile: None,
            },
        )
        .await
        .unwrap()
        .len(),
        3
    );
}

#[test_context(PgTokioContext)]
#[tokio::test]
async fn rolls_back_dynamic_writes(ctx: &mut PgTokioContext) {
    migrate(&ctx.client).await;
    let transaction = ctx.client.transaction().await.unwrap();

    assert_eq!(
        queries::set_user_phone(
            &transaction,
            queries::SetUserPhoneParams {
                new_phone: "rolled back",
                user_id: Some(1),
            },
        )
        .await
        .unwrap(),
        1
    );
    transaction.rollback().await.unwrap();

    let row = ctx
        .client
        .query_one("SELECT phone FROM users WHERE id = 1", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "111");
}
