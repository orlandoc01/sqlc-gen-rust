#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::queries;

    fn connection() -> rusqlite::Connection {
        rusqlite::Connection::open_in_memory().unwrap()
    }

    fn migrate(conn: &rusqlite::Connection) {
        conn.execute_batch(include_str!("../../sqlx-sqlite/schema.sql"))
            .unwrap();
        conn.execute_batch(
            "INSERT INTO users (id, email, phone) VALUES (1, 'alice@example.com', '111'), (2, 'bob@example.com', '222'), (3, 'carol@example.com', '333');
             INSERT INTO orders (id, user_id, created_at) VALUES (1, 1, '2024-01-01'), (2, 2, '2025-01-01');",
        )
        .unwrap();
    }

    fn params() -> queries::SearchUsersParams<'static> {
        queries::SearchUsersParams {
            row_limit: 100,
            ..Default::default()
        }
    }

    fn count_users_params() -> queries::CountUsersParams<'static> {
        Default::default()
    }

    #[test]
    fn searches_without_filters() {
        let conn = connection();
        migrate(&conn);

        assert_eq!(queries::search_users(&conn, params()).unwrap().len(), 3);
    }

    #[test]
    fn applies_each_scalar_filter() {
        let conn = connection();
        migrate(&conn);

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1]);

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                phone: Some("222"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn distinguishes_none_empty_and_populated_slices() {
        let conn = connection();
        migrate(&conn);

        assert_eq!(queries::search_users(&conn, params()).unwrap().len(), 3);
        let empty = [];
        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                ids: Some(&empty),
                ..params()
            },
        )
        .unwrap();
        assert!(users.is_empty());

        let ids = [1, 3];
        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                ids: Some(&ids),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(queries::dynfilter::nilable(&empty), None);
    }

    #[test]
    fn toggles_the_orders_block_and_its_filter() {
        let conn = connection();
        migrate(&conn);

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                has_orders: true,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 2]);

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                has_orders: true,
                orders_since: Some("2025-01-01"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn toggles_each_order_by_direction() {
        let conn = connection();
        migrate(&conn);

        let asc = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                id_asc: true,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(
            asc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );

        let desc = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                id_desc: true,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(
            desc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [3, 2, 1]
        );
    }

    #[test]
    fn counts_users_with_dynamic_filters() {
        let conn = connection();
        migrate(&conn);

        assert_eq!(
            queries::count_users(&conn, count_users_params())
                .unwrap()
                .total,
            3
        );
        assert_eq!(
            queries::count_users(
                &conn,
                queries::CountUsersParams {
                    email: Some("alice@example.com"),
                    ..count_users_params()
                },
            )
            .unwrap()
            .total,
            1
        );

        let empty = [];
        assert_eq!(
            queries::count_users(
                &conn,
                queries::CountUsersParams {
                    ids: Some(&empty),
                    ..count_users_params()
                },
            )
            .unwrap()
            .total,
            0
        );
        assert_eq!(
            queries::count_users(
                &conn,
                queries::CountUsersParams {
                    ids: None,
                    ..count_users_params()
                },
            )
            .unwrap()
            .total,
            3
        );
        assert!(
            queries::count_users_opt(&conn, count_users_params())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn touches_users_with_dynamic_filters() {
        let conn = connection();
        migrate(&conn);

        assert_eq!(queries::touch_users(&conn, Default::default()).unwrap(), 3);
        assert_eq!(
            queries::touch_users(
                &conn,
                queries::TouchUsersParams {
                    email: Some("alice@example.com"),
                    ..Default::default()
                },
            )
            .unwrap(),
            1
        );

        let empty = [];
        assert_eq!(
            queries::touch_users(
                &conn,
                queries::TouchUsersParams {
                    ids: Some(&empty),
                    ..Default::default()
                },
            )
            .unwrap(),
            0
        );
        assert_eq!(
            queries::touch_users(
                &conn,
                queries::TouchUsersParams {
                    ids: None,
                    ..Default::default()
                },
            )
            .unwrap(),
            3
        );
    }

    #[test]
    fn filters_by_owned_string_slices_and_repeated_scalars() {
        let conn = connection();
        migrate(&conn);
        let ids = |params| {
            queries::search_users_by_emails(&conn, params)
                .unwrap()
                .iter()
                .map(|user| user.id)
                .collect::<Vec<_>>()
        };
        let emails = [
            "alice@example.com".to_string(),
            "carol@example.com".to_string(),
        ];

        assert_eq!(ids(Default::default()), [1, 2, 3]);
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: Some(&emails),
                contact: None,
            }),
            [1, 3]
        );
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: None,
                contact: Some("222"),
            }),
            [2]
        );
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: None,
                contact: Some("bob@example.com"),
            }),
            [2]
        );
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: Some(&emails),
                contact: Some("111"),
            }),
            [1]
        );
        assert!(
            ids(queries::SearchUsersByEmailsParams {
                emails: Some(&[]),
                contact: None,
            })
            .is_empty()
        );
    }
}
