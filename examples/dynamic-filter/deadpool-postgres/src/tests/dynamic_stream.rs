use futures_util::TryStreamExt as _;
use test_context::test_context;
use test_utils::PgDeadpoolContext;

use crate::queries;

async fn migrate(client: &deadpool_postgres::Client) {
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

#[test_context(PgDeadpoolContext)]
#[tokio::test]
async fn drains_dynamic_streams_after_params_drop(ctx: &mut PgDeadpoolContext) {
    let client = ctx.pool.get().await.unwrap();
    migrate(&client).await;

    let active = queries::search_users_stream(
        &client,
        queries::SearchUsersParams {
            email: Some("alice@example.com"),
            ..params()
        },
    )
    .await
    .unwrap();
    futures_util::pin_mut!(active);
    assert_eq!(
        active
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1]
    );

    let inactive = queries::search_users_stream(&client, params())
        .await
        .unwrap();
    futures_util::pin_mut!(inactive);
    assert_eq!(
        inactive
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );

    let stream = {
        let profile = serde_json::json!({"tier": "gold"});
        queries::search_users_by_profile_stream(
            &client,
            queries::SearchUsersByProfileParams {
                email: None,
                profile: Some(profile),
            },
        )
        .await
        .unwrap()
    };
    futures_util::pin_mut!(stream);
    let rows = stream.try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(
        queries::SearchUsersByProfileRow::from_row(&rows[0])
            .unwrap()
            .email,
        "alice@example.com"
    );
}
