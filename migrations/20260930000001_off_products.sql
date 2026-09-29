-- Open Food Facts products imported from OFF's CSV export, and typo-tolerant product search

CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Read-only reference data; only `fittune-api import-off` writes here. A product becomes a
-- FitHealth product (`food_products`) when a user confirms it
CREATE TABLE off_products (
    barcode          TEXT PRIMARY KEY CHECK (barcode ~ '^[0-9]{8,14}$'),
    name             TEXT NOT NULL,
    brand            TEXT,
    main_category    TEXT,
    energy_kcal      DOUBLE PRECISION,
    protein_g        DOUBLE PRECISION,
    fat_g            DOUBLE PRECISION,
    carbs_g          DOUBLE PRECISION,
    saturated_fat_g  DOUBLE PRECISION,
    sugars_g         DOUBLE PRECISION,
    fiber_g          DOUBLE PRECISION,
    salt_g           DOUBLE PRECISION,
    serving_g        DOUBLE PRECISION,
    serving_name     TEXT,
    off_modified_at  TIMESTAMPTZ,
    imported_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The importer keeps all four core values or none
    CHECK ((energy_kcal IS NULL) = (protein_g IS NULL)
        AND (energy_kcal IS NULL) = (fat_g IS NULL)
        AND (energy_kcal IS NULL) = (carbs_g IS NULL))
);

-- Search only offers products that can be logged, so only those are indexed
CREATE INDEX off_products_name_trgm_idx ON off_products USING gin (lower(name) gin_trgm_ops)
    WHERE energy_kcal IS NOT NULL;
CREATE INDEX off_products_brand_trgm_idx ON off_products USING gin (lower(brand) gin_trgm_ops)
    WHERE energy_kcal IS NOT NULL;

CREATE INDEX food_products_name_trgm_idx ON food_products USING gin (lower(name) gin_trgm_ops);
CREATE INDEX food_products_brand_trgm_idx ON food_products USING gin (lower(brand) gin_trgm_ops);
