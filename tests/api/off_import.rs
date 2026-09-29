use std::io::Cursor;

use fittune_api::health::off::{import, row::Skip};
use sqlx::PgPool;

const HEADER: &str = "code\tlast_modified_t\tproduct_name\tbrands\tmain_category\tserving_size\tserving_quantity\tenergy_100g\tenergy-kcal_100g\tfat_100g\tsaturated-fat_100g\tcarbohydrates_100g\tsugars_100g\tfiber_100g\tproteins_100g\tsalt_100g";

fn csv(rows: &[&str]) -> Cursor<Vec<u8>> {
    let mut text = String::from(HEADER);
    for row in rows {
        text.push('\n');
        text.push_str(row);
    }
    text.push('\n');
    Cursor::new(text.into_bytes())
}

const OATS: &str = "5900259127761\t1700000000\tPłatki owsiane\tMelvit\ten:oat-flakes\t40 g\t40\t\t372\t7\t1.2\t60\t1\t10\t13\t0.01";
// The same product listed again, older: the newer listing wins
const OATS_OLD: &str =
    "5900259127761\t1600000000\tOld oats\tMelvit\t\t\t\t\t350\t6\t\t61\t\t\t12\t";
// UPC-A, stored as EAN-13; no nutrition
const SODA: &str = "036000291452\t1700000000\tCola\tBrand\t\t\t\t\t\t\t\t\t\t\t\t";
const BAD_BARCODE: &str = "5900259127762\t1700000000\tBroken\t\t\t\t\t\t100\t1\t\t1\t\t\t1\t";
const NO_NAME: &str = "4000417025005\t1700000000\t\t\t\t\t\t\t100\t1\t\t1\t\t\t1\t";
// Contains tabs and backslashes the COPY stream has to escape
const ODD_TEXT: &str =
    "96385074\t1700000000\tBack\\\\slash snack\tA\\\\B\t\t\t\t\t500\t30\t\t50\t\t\t8\t";

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn imports_filtered_products_and_reports_what_it_skipped(pool: PgPool) {
    let summary = import::import_reader(
        &pool,
        csv(&[
            OATS,
            OATS_OLD,
            SODA,
            BAD_BARCODE,
            NO_NAME,
            ODD_TEXT,
            "short\tline",
        ]),
    )
    .await
    .expect("import");
    assert_eq!(summary.read, 7);
    assert_eq!(summary.kept, 4);
    assert_eq!(summary.stored, 3);
    assert_eq!(summary.without_nutrition, 1);
    assert_eq!(summary.skipped.get(&Skip::Barcode), Some(&1));
    assert_eq!(summary.skipped.get(&Skip::Name), Some(&1));
    assert_eq!(summary.skipped.get(&Skip::Malformed), Some(&1));

    let oats: (String, Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT name, energy_kcal, serving_g FROM off_products WHERE barcode = '5900259127761'",
    )
    .fetch_one(&pool)
    .await
    .expect("oats");
    assert_eq!(oats, ("Płatki owsiane".to_owned(), Some(372.0), Some(40.0)));

    let soda: Option<f64> =
        sqlx::query_scalar("SELECT energy_kcal FROM off_products WHERE barcode = '0036000291452'")
            .fetch_one(&pool)
            .await
            .expect("soda");
    assert_eq!(soda, None);

    let odd: String =
        sqlx::query_scalar("SELECT name FROM off_products WHERE barcode = '96385074'")
            .fetch_one(&pool)
            .await
            .expect("odd");
    assert_eq!(odd, "Back\\\\slash snack");
}

#[sqlx::test(migrator = "fittune_api::db::MIGRATOR")]
async fn a_new_import_replaces_the_old_one_and_a_failed_one_keeps_it(pool: PgPool) {
    import::import_reader(&pool, csv(&[OATS, SODA]))
        .await
        .expect("first import");
    import::import_reader(&pool, csv(&[ODD_TEXT]))
        .await
        .expect("second import");
    let barcodes: Vec<String> =
        sqlx::query_scalar("SELECT barcode FROM off_products ORDER BY barcode")
            .fetch_all(&pool)
            .await
            .expect("barcodes");
    assert_eq!(barcodes, ["96385074"]);

    let broken = Cursor::new(b"not\ta\theader\n".to_vec());
    assert!(import::import_reader(&pool, broken).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM off_products")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}
