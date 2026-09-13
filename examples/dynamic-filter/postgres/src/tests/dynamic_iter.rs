use postgres::fallible_iterator::FallibleIterator as _;
use test_context::test_context;
use test_utils::PgSyncContext;

use super::queries;

fn migrate(client: &mut postgres::Client) {
    client
        .batch_execute(include_str!("../../schema.sql"))
        .unwrap();
    client
        .batch_execute(
            r#"INSERT INTO users (id, email, phone, profile) VALUES
                (1, 'alice@example.com', '111', '{"tier":"gold"}'),
                (2, 'bob@example.com', '222', '{"tier":"silver"}'),
                (3, 'carol@example.com', '333', '{"tier":"gold"}');"#,
        )
        .unwrap();
}

fn params() -> queries::SearchUsersParams<'static> {
    queries::SearchUsersParams {
        row_limit: 100,
        ..Default::default()
    }
}

#[test_context(PgSyncContext)]
#[test]
fn drains_dynamic_iterators_and_detaches_owned_params(ctx: &mut PgSyncContext) {
    let client = &mut ctx.client;
    migrate(client);

    let users = queries::search_users_iter(client, params())
        .unwrap()
        .collect::<Vec<_>>()
        .unwrap();
    assert_eq!(
        users
            .iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );

    let empty = [];
    assert!(
        queries::search_users_iter(
            client,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ids: Some(&empty),
                ..params()
            },
        )
        .unwrap()
        .collect::<Vec<_>>()
        .unwrap()
        .is_empty()
    );

    let ids = [1, 3];
    let users = queries::search_users_iter(
        client,
        queries::SearchUsersParams {
            email: Some("alice@example.com"),
            ids: Some(&ids),
            ..params()
        },
    )
    .unwrap()
    .collect::<Vec<_>>()
    .unwrap();
    assert_eq!(
        users
            .iter()
            .map(|row| row.get::<_, i64>(0))
            .collect::<Vec<_>>(),
        [1]
    );

    let users = {
        let profile = serde_json::json!({"tier": "gold"});
        queries::search_users_by_profile_iter(
            client,
            queries::SearchUsersByProfileParams {
                email: None,
                profile: Some(profile),
            },
        )
        .unwrap()
        .collect::<Vec<_>>()
        .unwrap()
    };
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
        .unwrap()
        .len(),
        3
    );
}

#[test_context(PgSyncContext)]
#[test]
fn rolls_back_dynamic_writes(ctx: &mut PgSyncContext) {
    migrate(&mut ctx.client);
    let mut transaction = ctx.client.transaction().unwrap();

    assert_eq!(
        queries::set_user_phone(
            &mut transaction,
            queries::SetUserPhoneParams {
                new_phone: "rolled back",
                user_id: Some(1),
            },
        )
        .unwrap(),
        1
    );
    transaction.rollback().unwrap();

    let row = ctx
        .client
        .query_one("SELECT phone FROM users WHERE id = 1", &[])
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "111");
}
