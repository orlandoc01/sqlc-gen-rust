#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use std::str::FromStr as _;
    use test_context::test_context;
    use test_utils::SqlxMysqlContext;

    use super::*;

    async fn migrate_db(pool: &sqlx::MySqlPool) {
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(pool)
            .await
            .unwrap();
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn maps_types(ctx: &mut SqlxMysqlContext) {
        let pool = &ctx.pool;
        migrate_db(pool).await;

        let blob_val = [1, 2, 3, 4, 5];
        queries::insert_mapping(
            pool,
            queries::InsertMappingParams {
                bool_val: true,
                tinyint_val: 1,
                smallint_val: 2,
                int_val: 3,
                int_nullable_val: Some(4),
                bigint_val: 5,
                float_val: 6.0,
                double_val: 7.0,
                text_val: "8",
                blob_val: &blob_val,
                timestamp_val: chrono::Utc.with_ymd_and_hms(2025, 1, 23, 4, 5, 6).unwrap(),
                datetime_val: chrono::NaiveDateTime::from_str("2025-01-23T04:05:06").unwrap(),
                date_val: chrono::NaiveDate::from_ymd_opt(2025, 1, 23).unwrap(),
                time_val: sqlx::mysql::types::MySqlTime::ZERO,
                json_val: serde_json::json!({ "type": "json" }),
            },
        )
        .await
        .unwrap();

        queries::get_mapping(pool).await.unwrap();
    }
}
