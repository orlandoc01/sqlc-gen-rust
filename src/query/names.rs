//! Row field naming from sqlc column metadata.

use crate::plugin;

/// 次の命名規則で、カラム名を生成する
///
/// 1. テーブル名とカラム名が両方とも空の時: column_1, column_2...
/// 2. 同じカラム名が存在しないとき: column_name
/// 3. 同じカラム名が存在し、テーブル名が異なる時: table_column
/// 4. テーブル名もカラム名も同一の時: table_column_1, table_column_2
pub(super) fn generate_column_names<'a, I>(columns: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a plugin::Column>,
{
    // Step 1: カラム情報を収集
    let column_info: Vec<_> = columns
        .into_iter()
        .map(|column| {
            let table_name = column.table.as_ref().map(|t| t.name.as_str()).unwrap_or("");
            let column_name = column.name.as_str();
            (table_name, column_name)
        })
        .collect();

    // Step 2: 空でないカラム名の出現回数をカウント
    let mut column_name_counts = std::collections::HashMap::new();
    for &(_, column_name) in &column_info {
        if !column_name.is_empty() {
            *column_name_counts.entry(column_name).or_insert(0) += 1;
        }
    }

    // Step 3: 各カラムの基本名を決定
    let mut base_names = Vec::new();
    for (i, &(table_name, column_name)) in column_info.iter().enumerate() {
        let base_name = match (table_name.is_empty(), column_name.is_empty()) {
            (true, true) => {
                // Rule 1: テーブル名とカラム名が両方とも空
                format!("column_{}", i + 1)
            }
            (_, true) => {
                // カラム名が空の場合（テーブル名の有無は関係なし）
                format!("column_{}", i + 1)
            }
            (_, false) => {
                // カラム名が存在する場合
                let count = column_name_counts.get(column_name).unwrap_or(&0);
                if *count == 1 {
                    // Rule 2: 同じカラム名が存在しない
                    column_name.to_string()
                } else {
                    // Rule 3: 同じカラム名が存在する
                    if table_name.is_empty() {
                        column_name.to_string()
                    } else {
                        format!("{table_name}_{column_name}")
                    }
                }
            }
        };
        base_names.push(base_name);
    }

    // Step 4: 最終的な名前の重複を解決（Rule 4）
    // まず重複する基本名を特定
    let mut base_name_counts = std::collections::HashMap::new();
    for base_name in &base_names {
        *base_name_counts.entry(base_name.clone()).or_insert(0) += 1;
    }

    let mut final_names = Vec::new();
    let mut name_occurrence_counts = std::collections::HashMap::new();

    for base_name in base_names {
        let total_count = base_name_counts.get(&base_name).unwrap_or(&1);
        let occurrence_count = name_occurrence_counts.entry(base_name.clone()).or_insert(0);
        *occurrence_count += 1;

        let final_name = if *total_count == 1 {
            // 重複がない場合はそのまま
            base_name
        } else {
            // 重複がある場合は最初から連番を付ける
            format!("{base_name}_{occurrence_count}")
        };
        final_names.push(final_name);
    }

    final_names
}
