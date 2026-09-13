#[allow(dead_code)]
mod queries;

#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "complex")]
struct Complex {
    r: f64,
    i: f64,
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr},
        str::FromStr as _,
    };

    use super::*;
    use chrono::TimeZone;
    use sqlx::postgres::types::*;
    use test_context::test_context;
    use test_utils::SqlxPgContext;

    async fn migrate_db(pool: &sqlx::PgPool) {
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(pool)
            .await
            .unwrap();
    }

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn maps_types(ctx: &mut SqlxPgContext) {
        let pool = &ctx.pool;
        migrate_db(pool).await;

        let bool_array_val = [true, false];
        let bytea_val = [1, 2, 3, 4, 5];
        let mut hstore_val = PgHstore::default();
        hstore_val.insert("type".to_string(), Some("hstore".to_string()));
        let timestamp_val = chrono::NaiveDateTime::from_str("2025-01-23T04:05:06").unwrap();
        let timestamptz_val = chrono::Utc.with_ymd_and_hms(2025, 1, 23, 4, 5, 6).unwrap();
        let date_val = chrono::NaiveDate::from_ymd_opt(2025, 1, 23).unwrap();
        let time_val = chrono::NaiveTime::from_hms_opt(1, 23, 45).unwrap();
        let inet_val = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let json_val = serde_json::json!({"type": "json"});
        let jsonb_val = serde_json::json!({"type": "jsonb"});

        queries::insert_mapping(
            pool,
            queries::InsertMappingParams {
                bool_val: true,
                bool_array_val: &bool_array_val,
                char_val: 1,
                smallint_val: 2,
                int_val: 3,
                int_nullable_val: Some(4),
                oid_val: Oid(5),
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
                inet_val,
                json_val,
                jsonb_val,
                uuid_val: "366dacaf-6812-4f94-8d20-25f5e7f4981c".parse().unwrap(),
                enum_val: queries::Mood::Sad,
                composite_val: Complex { r: 12.3, i: 45.6 },
                money_val: PgMoney(12345),
                ltree_val: PgLTree::new(),
                lquery_val: PgLQuery::from(vec![PgLQueryLevel::Star(None, None)]),
                cube_val: PgCube::Point(12.34),
                point_val: PgPoint { x: 12.34, y: 56.78 },
                line_val: PgLine {
                    a: 1.0,
                    b: 2.0,
                    c: 0.0,
                },
                lseg_val: PgLSeg {
                    start_x: 1.0,
                    start_y: 2.0,
                    end_x: 3.0,
                    end_y: 4.0,
                },
                box_val: PgBox {
                    upper_right_x: 1.0,
                    upper_right_y: 2.0,
                    lower_left_x: 3.0,
                    lower_left_y: 4.0,
                },
                path_val: PgPath {
                    closed: true,
                    points: vec![PgPoint { x: 0.0, y: 0.0 }, PgPoint { x: 1.0, y: 1.0 }],
                },
                polygon_val: PgPolygon {
                    points: vec![
                        PgPoint { x: 0.0, y: 0.0 },
                        PgPoint { x: 1.0, y: 1.0 },
                        PgPoint { x: 2.0, y: 0.0 },
                    ],
                },
                circle_val: PgCircle {
                    x: 1.0,
                    y: 2.0,
                    radius: 1.0,
                },
            },
        )
        .await
        .unwrap();

        queries::get_mapping(pool).await.unwrap();
    }
}
