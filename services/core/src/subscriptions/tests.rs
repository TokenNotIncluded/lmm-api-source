use sqlx::PgPool;

// Both subscription grants and charging admission must agree even after the
// halfway point of a DB second. A plain PostgreSQL `::bigint` cast rounds up.
#[sqlx::test]
async fn subscription_clock_floors_the_same_db_second_as_charging(pool: PgPool) {
    let mut connection = pool.acquire().await.unwrap();
    for _ in 0..10 {
        sqlx::query("SELECT pg_sleep(mod(1.65 - mod(extract(epoch FROM clock_timestamp()), 1), 1)::double precision)")
            .execute(&mut *connection)
            .await
            .unwrap();
        let (before, fraction): (i64, f64) = sqlx::query_as(
            "SELECT floor(t)::bigint,(t-floor(t))::double precision FROM (SELECT extract(epoch FROM clock_timestamp()) AS t OFFSET 0) sample",
        )
        .fetch_one(&mut *connection)
        .await
        .unwrap();
        let actual = super::now(&mut connection).await.unwrap();
        let previously_rounded: i64 =
            sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        let after: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        // Retry if scheduling crossed a second boundary, or did not reach the
        // latter half; never pass without observing the regression condition.
        if fraction >= 0.5 && before == after {
            assert_eq!(previously_rounded, before + 1);
            assert_eq!(actual, before);
            return;
        }
    }
    panic!("could not observe the latter half of a stable database second");
}
