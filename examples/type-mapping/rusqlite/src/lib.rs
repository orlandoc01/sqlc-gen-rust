#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;
    use test_context::test_context;
    use test_utils::RusqliteContext;

    fn migrate_db(conn: &rusqlite::Connection) {
        conn.execute_batch(include_str!("../../sqlx-sqlite/schema.sql"))
            .unwrap();
    }

    /// Every column holds a distinct value, so a swapped ordinal between same-typed
    /// columns fails here.
    fn assert_round_trip(row: &queries::GetMappingRow, params: &queries::InsertMappingParams<'_>) {
        assert_eq!(row.aff_integer_val, params.aff_integer_val);
        assert_eq!(row.aff_real_val, params.aff_real_val);
        assert_eq!(row.aff_text_val, params.aff_text_val);
        assert_eq!(row.aff_blob_val, params.aff_blob_val);
        assert_eq!(row.int_val, params.int_val);
        assert_eq!(row.integer_val, params.integer_val);
        assert_eq!(row.tinyint_val, params.tinyint_val);
        assert_eq!(row.smallint_val, params.smallint_val);
        assert_eq!(row.mediumint_val, params.mediumint_val);
        assert_eq!(row.bigint_val, params.bigint_val);
        assert_eq!(row.unsigned_big_int_val, params.unsigned_big_int_val);
        assert_eq!(row.int_2_val, params.int_2_val);
        assert_eq!(row.int_8_val, params.int_8_val);
        assert_eq!(row.character_20_val, params.character_20_val);
        assert_eq!(row.varchar_255_val, params.varchar_255_val);
        assert_eq!(row.varying_char_255_val, params.varying_char_255_val);
        assert_eq!(row.nchar_55_val, params.nchar_55_val);
        assert_eq!(row.native_char_70_val, params.native_char_70_val);
        assert_eq!(row.nvarchar_100_val, params.nvarchar_100_val);
        assert_eq!(row.text_val, params.text_val);
        assert_eq!(row.clob_val, params.clob_val);
        assert_eq!(row.real_val, params.real_val);
        assert_eq!(row.double_val, params.double_val);
        assert_eq!(row.double_precision_val, params.double_precision_val);
        assert_eq!(row.float_val, params.float_val);
        assert_eq!(row.numeric_val, params.numeric_val);
        assert_eq!(row.decimal_10_5_val, params.decimal_10_5_val);
        assert_eq!(row.boolean_val, params.boolean_val);
        assert_eq!(row.date_val, params.date_val);
        assert_eq!(row.time_val, params.time_val);
        assert_eq!(row.datetime_val, params.datetime_val);
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn round_trips_distinct_values(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);

        let params = queries::InsertMappingParams {
            aff_integer_val: 1,
            aff_real_val: 2.5,
            aff_text_val: "3",
            aff_blob_val: &[1, 2, 3, 4, 5],
            int_val: 5,
            integer_val: 6,
            tinyint_val: 7,
            smallint_val: 8,
            mediumint_val: 9,
            bigint_val: 10,
            unsigned_big_int_val: 11,
            int_2_val: 12,
            int_8_val: 13,
            character_20_val: "14",
            varchar_255_val: "15",
            varying_char_255_val: "16",
            nchar_55_val: "17",
            native_char_70_val: "18",
            nvarchar_100_val: "19",
            text_val: "20",
            clob_val: "21",
            real_val: 22.25,
            double_val: 23.5,
            double_precision_val: 24.75,
            float_val: 25.125,
            numeric_val: 26.1,
            decimal_10_5_val: 27.1,
            boolean_val: true,
            date_val: chrono::NaiveDate::from_ymd_opt(2025, 1, 23).unwrap(),
            time_val: chrono::NaiveTime::from_hms_opt(1, 23, 45).unwrap(),
            datetime_val: chrono::NaiveDateTime::from_str("2025-01-23T04:05:06").unwrap(),
        };
        queries::insert_mapping(conn, params.clone()).unwrap();
        let row = queries::get_mapping(conn).unwrap();
        assert_round_trip(&row, &params);

        let ids = queries::get_mapping_by_client_and_statement(conn, "3", "20").unwrap();
        assert_eq!(ids.len(), 1);
        assert!(
            queries::get_mapping_by_client_and_statement(conn, "20", "3")
                .unwrap()
                .is_empty()
        );
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn round_trips_boundary_values(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);

        let params = queries::InsertMappingParams {
            aff_integer_val: i64::MIN,
            aff_real_val: -0.5,
            aff_text_val: "",
            aff_blob_val: &[],
            int_val: -1,
            integer_val: i64::MAX,
            tinyint_val: i8::MIN,
            smallint_val: i16::MAX,
            mediumint_val: i32::MIN,
            bigint_val: -2,
            unsigned_big_int_val: -3,
            int_2_val: i16::MIN,
            int_8_val: -4,
            character_20_val: "ünïcödé",
            varchar_255_val: " leading and trailing ",
            varying_char_255_val: "quote's",
            nchar_55_val: "tab\tnewline\n",
            native_char_70_val: "a",
            nvarchar_100_val: "b",
            text_val: "c",
            clob_val: "d",
            real_val: f64::MAX,
            double_val: f64::MIN_POSITIVE,
            double_precision_val: -1e300,
            float_val: 0.0,
            // Integral NUMERIC values are stored as INTEGER; rusqlite widens them back to f64.
            numeric_val: 26.0,
            decimal_10_5_val: -27.0,
            boolean_val: false,
            date_val: chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            time_val: chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
            datetime_val: chrono::NaiveDateTime::from_str("1999-12-31T23:59:59").unwrap(),
        };
        queries::insert_mapping(conn, params.clone()).unwrap();
        let row = queries::get_mapping(conn).unwrap();
        assert_round_trip(&row, &params);
    }
}
