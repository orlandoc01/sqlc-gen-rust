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
    use test_context::test_context;
    use test_utils::PgDeadpoolContext;

    use super::*;

    async fn migrate_db(client: &deadpool_postgres::Client) {
        client
            .batch_execute(include_str!("../schema.sql"))
            .await
            .unwrap();
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn maps_types(ctx: &mut PgDeadpoolContext) {
        let client = ctx.pool.get().await.unwrap();
        migrate_db(&client).await;

        let bool_array_val = [true, false];
        let bytea_val = [1, 2, 3, 4, 5];
        let mut hstore_val = HashMap::new();
        hstore_val.insert("type".to_string(), Some("hstore".to_string()));
        let timestamp_val = chrono::NaiveDateTime::from_str("2025-01-23T04:05:06").unwrap();
        let timestamptz_val = chrono::Utc.with_ymd_and_hms(2025, 1, 23, 4, 5, 6).unwrap();
        let date_val = chrono::NaiveDate::from_ymd_opt(2025, 1, 23).unwrap();
        let time_val = chrono::NaiveTime::from_hms_opt(1, 23, 45).unwrap();
        let statement = queries::prepare_insert_mapping(&client).await.unwrap();
        queries::insert_mapping_with(
            &client,
            &statement,
            queries::InsertMappingParams {
                bool_val: true,
                bool_array_val: &bool_array_val,
                char_val: 1,
                smallint_val: 2,
                int_val: 3,
                int_nullable_val: Some(4),
                oid_val: 5,
                bigint_val: 6,
                real_val: 7.0,
                double_val: 8.0,
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
        .await
        .unwrap();

        let mapping = queries::get_mapping(&client).await.unwrap();

        assert!(mapping.bool_val);
        assert_eq!(mapping.bool_array_val, bool_array_val);
        assert_eq!(mapping.int_val, 3);
        assert_eq!(mapping.int_nullable_val, Some(4));
        assert_eq!(mapping.text_val, "9");
        assert_eq!(mapping.text_nullable_val, Some("10".to_string()));
        assert_eq!(mapping.bytea_val, bytea_val);
        assert_eq!(mapping.oid_val, 5);
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
    }
}
