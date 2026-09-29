//! `fittune-api import-off <file>`: loads OFF's CSV export into `off_products`.
//!
//! The file (about 13 GB) is read line by line and streamed into a staging table with `COPY`, so
//! it never sits in memory. The staging rows replace `off_products` in the same transaction:
//! readers see the old data until the commit, and a failed import leaves it untouched

use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::{Context, Result};
use sqlx::PgPool;

use super::row::{self, Columns, OffRow, Skip};

/// Rows are sent to Postgres in chunks of about this many bytes
const CHUNK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct Summary {
    /// Product lines in the file, not counting the header
    pub read: u64,
    /// Lines that mapped to a product
    pub kept: u64,
    /// Products stored after keeping the newest listing of each barcode
    pub stored: u64,
    /// Kept lines without usable nutrition values
    pub without_nutrition: u64,
    pub skipped: HashMap<Skip, u64>,
}

pub async fn import_file(db: &PgPool, path: &Path) -> Result<Summary> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    import_reader(db, BufReader::with_capacity(1024 * 1024, file)).await
}

pub async fn import_reader(db: &PgPool, mut reader: impl BufRead) -> Result<Summary> {
    let mut line = Vec::new();
    reader
        .read_until(b'\n', &mut line)
        .context("cannot read the header")?;
    let columns = Columns::from_header(&String::from_utf8_lossy(&line))?;

    let mut tx = db.begin().await?;
    sqlx::query(
        "CREATE TEMP TABLE off_staging (LIKE off_products INCLUDING DEFAULTS) ON COMMIT DROP",
    )
    .execute(&mut *tx)
    .await?;

    let mut summary = Summary::default();
    let mut chunk = String::with_capacity(CHUNK_BYTES + 64 * 1024);
    loop {
        line.clear();
        // Reading the file is blocking; this is a one-off command, not a request handler
        if reader
            .read_until(b'\n', &mut line)
            .context("cannot read the file")?
            == 0
        {
            break;
        }
        let text = String::from_utf8_lossy(&line);
        if text.trim().is_empty() {
            continue;
        }
        summary.read += 1;
        match row::parse(&columns, &text) {
            Ok(product) => {
                summary.kept += 1;
                if product.nutrients.is_none() {
                    summary.without_nutrition += 1;
                }
                write_copy_line(&mut chunk, &product);
            }
            Err(skip) => *summary.skipped.entry(skip).or_default() += 1,
        }
        if chunk.len() >= CHUNK_BYTES {
            copy_chunk(&mut tx, &chunk).await?;
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        copy_chunk(&mut tx, &chunk).await?;
    }

    sqlx::query("TRUNCATE off_products")
        .execute(&mut *tx)
        .await?;
    // A barcode listed more than once (EAN-13 and UPC-A of one product) keeps its newest listing
    summary.stored = sqlx::query(
        "INSERT INTO off_products
         SELECT DISTINCT ON (barcode) * FROM off_staging
         ORDER BY barcode, off_modified_at DESC NULLS LAST",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(summary)
}

async fn copy_chunk(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, chunk: &str) -> Result<()> {
    let mut copy = tx
        .copy_in_raw(
            "COPY off_staging (barcode, name, brand, main_category, energy_kcal, protein_g, fat_g,
                carbs_g, saturated_fat_g, sugars_g, fiber_g, salt_g, serving_amount, serving_name,
                off_modified_at, unit) FROM STDIN",
        )
        .await?;
    copy.send(chunk.as_bytes()).await?;
    copy.finish().await?;
    Ok(())
}

/// One row in `COPY ... FROM STDIN` text format: tab separated, `\N` for null
fn write_copy_line(out: &mut String, product: &OffRow) {
    let n = product.nutrients;
    let fields: [Option<String>; 16] = [
        Some(product.barcode.clone()),
        Some(product.name.clone()),
        product.brand.clone(),
        product.main_category.clone(),
        n.map(|n| n.energy_kcal.to_string()),
        n.map(|n| n.protein_g.to_string()),
        n.map(|n| n.fat_g.to_string()),
        n.map(|n| n.carbs_g.to_string()),
        n.and_then(|n| n.saturated_fat_g).map(|v| v.to_string()),
        n.and_then(|n| n.sugars_g).map(|v| v.to_string()),
        n.and_then(|n| n.fiber_g).map(|v| v.to_string()),
        n.and_then(|n| n.salt_g).map(|v| v.to_string()),
        product.serving_amount.map(|v| v.to_string()),
        product.serving_name.clone(),
        product.modified_at.map(|t| t.to_rfc3339()),
        Some(product.unit.as_str().to_owned()),
    ];
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push('\t');
        }
        match field {
            None => out.push_str("\\N"),
            Some(value) => escape_into(out, value),
        }
    }
    out.push('\n');
}

fn escape_into(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
}
