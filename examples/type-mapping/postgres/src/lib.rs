#[allow(dead_code)]
mod queries;

#[derive(Debug, Clone, Copy, postgres_types::ToSql, postgres_types::FromSql)]
#[postgres(name = "complex")]
struct Complex {
    r: f64,
    i: f64,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        net::{IpAddr, Ipv4Addr},
        str::FromStr as _,
    };

    use chrono::TimeZone;
    use postgres::fallible_iterator::FallibleIterator as _;
    use test_context::test_context;
    use test_utils::PgSyncContext;

    use super::*;

    fn migrate_db(client: &mut postgres::Client) {
        client.batch_execute(include_str!("../schema.sql")).unwrap();
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn maps_types(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);

        let bool_array_val = [true, false];
        let bytea_val = [1, 2, 3, 4, 5];
        let mut hstore_val = HashMap::new();
        hstore_val.insert("type".to_string(), Some("hstore".to_string()));
        let timestamp_val = chrono::NaiveDateTime::from_str("2025-01-23T04:05:06").unwrap();
        let timestamptz_val = chrono::Utc.with_ymd_and_hms(2025, 1, 23, 4, 5, 6).unwrap();
        let date_val = chrono::NaiveDate::from_ymd_opt(2025, 1, 23).unwrap();
        let time_val = chrono::NaiveTime::from_hms_opt(1, 23, 45).unwrap();
        let statement = queries::prepare_insert_mapping(client).unwrap();
        queries::insert_mapping_with(
            client,
            &statement,
            queries::InsertMappingParams {
                bool_val: true,
                bool_array_val: &bool_array_val,
                char_val: -1,
                smallint_val: i16::MIN,
                int_val: i32::MIN,
                int_nullable_val: Some(-42),
                oid_val: u32::MAX,
                bigint_val: i64::MIN,
                real_val: -0.5,
                double_val: 0.25,
                text_val: "9",
                text_nullable_val: Some("10"),
                bytea_val: &bytea_val,
                hstore_val,
                timestamp_val,
                timestamptz_val,
                date_val,
                time_val,
                inet_val: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                json_val: serde_json::json!({"type": "json"}),
                jsonb_val: serde_json::json!({"type": "jsonb"}),
                uuid_val: "366dacaf-6812-4f94-8d20-25f5e7f4981c".parse().unwrap(),
                enum_val: queries::Mood::Sad,
                composite_val: Complex { r: 12.3, i: 45.6 },
            },
        )
        .unwrap();

        let mapping = queries::get_mapping(client).unwrap();

        assert!(mapping.bool_val);
        assert_eq!(mapping.bool_array_val, bool_array_val);
        assert_eq!(mapping.char_val, -1);
        assert_eq!(mapping.smallint_val, i16::MIN);
        assert_eq!(mapping.int_val, i32::MIN);
        assert_eq!(mapping.int_nullable_val, Some(-42));
        assert_eq!(mapping.oid_val, u32::MAX);
        assert_eq!(mapping.bigint_val, i64::MIN);
        assert_eq!(mapping.real_val, -0.5);
        assert_eq!(mapping.double_val, 0.25);
        assert_eq!(mapping.text_val, "9");
        assert_eq!(mapping.text_nullable_val, Some("10".to_string()));
        assert_eq!(mapping.bytea_val, bytea_val);
        assert_eq!(mapping.inet_val, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
        assert_eq!(mapping.json_val, serde_json::json!({"type": "json"}));
        assert_eq!(mapping.jsonb_val, serde_json::json!({"type": "jsonb"}));
        assert_eq!(
            mapping.uuid_val,
            "366dacaf-6812-4f94-8d20-25f5e7f4981c"
                .parse::<uuid::Uuid>()
                .unwrap()
        );
        assert_eq!(
            mapping.hstore_val,
            HashMap::from([("type".to_string(), Some("hstore".to_string()))])
        );
        assert_eq!(
            mapping.timestamp_val,
            chrono::NaiveDateTime::from_str("2025-01-23T04:05:06").unwrap()
        );
        assert_eq!(
            mapping.timestamptz_val,
            chrono::Utc.with_ymd_and_hms(2025, 1, 23, 4, 5, 6).unwrap()
        );
        assert_eq!(
            mapping.date_val,
            chrono::NaiveDate::from_ymd_opt(2025, 1, 23).unwrap()
        );
        assert_eq!(
            mapping.time_val,
            chrono::NaiveTime::from_hms_opt(1, 23, 45).unwrap()
        );
        assert!(matches!(mapping.enum_val, queries::Mood::Sad));
        assert_eq!(mapping.composite_val.r, 12.3);
        assert_eq!(mapping.composite_val.i, 45.6);

        client.execute("DELETE FROM mapping", &[]).unwrap();
        let empty_bool_array = [];
        queries::insert_mapping(
            client,
            queries::InsertMappingParams {
                bool_val: false,
                bool_array_val: &empty_bool_array,
                char_val: 0,
                smallint_val: 0,
                int_val: 0,
                int_nullable_val: None,
                oid_val: 0,
                bigint_val: 0,
                real_val: 0.5,
                double_val: -0.25,
                text_val: "nullable",
                text_nullable_val: None,
                bytea_val: &[],
                hstore_val: HashMap::from([("missing".to_string(), None)]),
                timestamp_val,
                timestamptz_val,
                date_val,
                time_val,
                inet_val: IpAddr::V4(Ipv4Addr::LOCALHOST),
                json_val: serde_json::json!({"type": "json"}),
                jsonb_val: serde_json::json!({"type": "jsonb"}),
                uuid_val: "366dacaf-6812-4f94-8d20-25f5e7f4981c".parse().unwrap(),
                enum_val: queries::Mood::Happy,
                composite_val: Complex { r: -0.5, i: 0.25 },
            },
        )
        .unwrap();
        let mapping = queries::get_mapping(client).unwrap();
        assert!(!mapping.bool_val);
        assert_eq!(mapping.bool_array_val, empty_bool_array);
        assert_eq!(mapping.int_nullable_val, None);
        assert_eq!(mapping.text_nullable_val, None);
        assert_eq!(
            mapping.hstore_val,
            HashMap::from([("missing".to_string(), None)])
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn enum_named_s_works_with_statement_helpers(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
        client
            .execute(
                "INSERT INTO state_mappings (id, state) VALUES (1, $1), (2, $2)",
                &[&queries::S::A, &queries::S::B],
            )
            .unwrap();

        assert_eq!(
            queries::get_by_state_with(client, queries::GET_BY_STATE, queries::S::A)
                .unwrap()
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            [1]
        );
        let statement = queries::prepare_get_by_state(client).unwrap();
        assert_eq!(
            queries::get_by_state_with(client, &statement, queries::S::B)
                .unwrap()
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(
            queries::get_by_state_iter_with(client, queries::GET_BY_STATE, queries::S::A)
                .unwrap()
                .collect::<Vec<_>>()
                .unwrap()
                .iter()
                .map(|row| row.get::<_, i64>(0))
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(
            queries::get_one_by_state_opt_with(client, queries::GET_ONE_BY_STATE, queries::S::B,)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            queries::get_by_state_with_minimum_id(
                client,
                queries::GetByStateWithMinimumIdParams {
                    state: queries::S::A,
                    minimum_id: 1,
                },
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn enum_named_sync_works_with_static_and_dynamic_queries(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
        client
            .execute(
                "INSERT INTO sync_mappings (id, state) VALUES (1, $1)",
                &[&queries::Sync::Ready],
            )
            .unwrap();

        let entry =
            queries::get_sync_entry_with(client, queries::GET_SYNC_ENTRY, queries::Sync::Ready)
                .unwrap();
        assert_eq!(entry.id, 1);
        assert!(matches!(entry.state, queries::Sync::Ready));
        let entries = queries::search_sync_entries(
            client,
            queries::SearchSyncEntriesParams {
                state: Some(queries::Sync::Ready),
            },
        )
        .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            [1]
        );
    }
}
