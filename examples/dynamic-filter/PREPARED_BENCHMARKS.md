# `dynfilters.prepared` timing

Release-mode measurements of the ignored `measures_prepared_statement_reuse` test in each
caching example crate. Every cell is the median of three runs. One pooled connection; the
migration and seed run before any timer starts.

- `uncached`: the generated cache-skipped twin of `SearchUsersByPattern` (same SQL text, in
  `dynfilters.prepared_skip`), so it executes with `persistent(false)`, `prepare(&sql)`, or
  `sql.as_str()` and nothing is warmed.
- `cache on first use`: the eligible query without a warm-up; the cache fills as shapes execute.
- `warmed`: `prepare_dynfilter_variants` runs in `after_connect` / `post_create` / before the
  first query; "connect+warm" includes it.
- `first` is the first execution of one shape, `hot` the per-call mean of 200 (rusqlite: 2000)
  repeats of that shape, `mixed` the per-call mean over the same number of cycles through the
  fixture's four shapes. "cached" is the client cache size after the workload; "warmed shapes"
  (in the backend column) is `DYNFILTER_VARIANT_COUNT`, the number the aggregate prepares.

| backend (warmed shapes) | mode | capacity | cached | connect+warm | first | hot/call | mixed/call |
| --- | --- | --- | --- | --- | --- | --- | --- |
| sqlx-postgres (123) | uncached (skipped twin) | 128 | 0 | 3.43ms | 1.03ms | 133.0µs | 77.0µs |
| sqlx-postgres (123) | cache on first use | 128 | 4 | 3.12ms | 969.9µs | 34.7µs | 31.4µs |
| sqlx-postgres (123) | warmed | 128 | 119 | 8.19ms | 353.5µs | 27.9µs | 27.2µs |
| sqlx-mysql (34) | uncached (skipped twin) | 128 | 0 | 1.89ms | 253.3µs | 149.1µs | 76.6µs |
| sqlx-mysql (34) | cache on first use | 128 | 4 | 1.81ms | 228.8µs | 42.2µs | 36.8µs |
| sqlx-mysql (34) | warmed | 128 | 34 | 3.43ms | 116.3µs | 46.1µs | 38.0µs |
| sqlx-sqlite (45) | uncached (skipped twin) | 128 | 0 | 181.1µs | 121.3µs | 18.3µs | 15.3µs |
| sqlx-sqlite (45) | cache on first use | 128 | 4 | 154.8µs | 110.6µs | 16.1µs | 13.1µs |
| sqlx-sqlite (45) | warmed | 128 | 45 | 1.44ms | 36.3µs | 13.7µs | 12.2µs |
| rusqlite (45) | uncached (skipped twin) | 128 | - | 331ns | 41.4µs | 4.7µs | 3.3µs |
| rusqlite (45) | cache on first use | 128 | - | 64ns | 22.1µs | 869ns | 915ns |
| rusqlite (45) | warmed | 128 | - | 385.0µs | 3.4µs | 861ns | 882ns |
| deadpool-postgres (122) | uncached (skipped twin) | unbounded | 0 | 3.51ms | 1.21ms | 106.8µs | 85.8µs |
| deadpool-postgres (122) | cache on first use | unbounded | 4 | 3.49ms | 1.30ms | 83.8µs | 43.0µs |
| deadpool-postgres (122) | warmed | unbounded | 120 | 11.99ms | 419.9µs | 37.9µs | 36.9µs |

Reading: on the network backends a cached shape costs roughly a third of an uncached call and
the warm-up removes the first-execution prepare (about 0.7 ms on PostgreSQL); warming ~120
PostgreSQL shapes adds 5-8 ms to connection creation. rusqlite has no client-visible cache size
and its warm-up is a local prepare of 45 statements. sqlx-sqlite's "warmed" connect time includes
opening the file-backed database through the hooked pool.

## How it was produced

```sh
docker run -d --rm --name sqlcrust-pg -p 127.0.0.1:5433:5432 -e POSTGRES_USER=root -e POSTGRES_PASSWORD=password -e POSTGRES_DB=app postgres:17.0-bookworm
docker run -d --rm --name sqlcrust-mysql -p 127.0.0.1:3307:3306 -e MYSQL_ROOT_PASSWORD=password -e MYSQL_DATABASE=app mysql:9.4
export POSTGRES_DATABASE_URL=postgres://root:password@127.0.0.1:5433/app
export MYSQL_DATABASE_URL=mysql://root:password@127.0.0.1:3307/app
for run in 1 2 3; do
  for p in dynamic-filter-sqlx-postgres dynamic-filter-sqlx-mysql dynamic-filter-sqlx-sqlite dynamic-filter-rusqlite dynamic-filter-deadpool-postgres; do
    cargo test --release -p $p measures_prepared_statement_reuse -- --ignored --nocapture
  done
done
```

