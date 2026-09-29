-- FitHealth diary: shared products, per-user meals and entries, body profile and weights

-- Shared by every user. Values are per 100 g
CREATE TABLE food_products (
    id               UUID PRIMARY KEY,
    name             TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    brand            TEXT CHECK (length(brand) BETWEEN 1 AND 80),
    barcode          TEXT UNIQUE CHECK (barcode ~ '^[0-9]{8,14}$'),
    energy_kcal      DOUBLE PRECISION NOT NULL CHECK (energy_kcal >= 0 AND energy_kcal <= 900),
    protein_g        DOUBLE PRECISION NOT NULL CHECK (protein_g >= 0 AND protein_g <= 100),
    fat_g            DOUBLE PRECISION NOT NULL CHECK (fat_g >= 0 AND fat_g <= 100),
    carbs_g          DOUBLE PRECISION NOT NULL CHECK (carbs_g >= 0 AND carbs_g <= 100),
    saturated_fat_g  DOUBLE PRECISION CHECK (saturated_fat_g >= 0 AND saturated_fat_g <= fat_g),
    sugars_g         DOUBLE PRECISION CHECK (sugars_g >= 0 AND sugars_g <= carbs_g),
    fiber_g          DOUBLE PRECISION CHECK (fiber_g >= 0 AND fiber_g <= 100),
    salt_g           DOUBLE PRECISION CHECK (salt_g >= 0 AND salt_g <= 100),
    serving_g        DOUBLE PRECISION CHECK (serving_g > 0 AND serving_g <= 2000),
    serving_name     TEXT CHECK (length(serving_name) BETWEEN 1 AND 40),
    source           TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'off', 'ocr', 'ai')),
    created_by       UUID REFERENCES users (id) ON DELETE SET NULL,
    updated_by       UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (protein_g + fat_g + carbs_g <= 100)
);

CREATE INDEX food_products_name_idx ON food_products (lower(name));

-- A user's meals, in their order. Archived meals keep their past entries
CREATE TABLE diary_meals (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 40),
    position     INTEGER NOT NULL,
    archived_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX diary_meals_user_idx ON diary_meals (user_id, position);

-- The product's name and values are copied in, so edits to a shared product never rewrite
-- someone's history
CREATE TABLE diary_entries (
    id               UUID PRIMARY KEY,
    user_id          UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    date             DATE NOT NULL,
    meal_id          UUID NOT NULL REFERENCES diary_meals (id),
    product_id       UUID REFERENCES food_products (id) ON DELETE SET NULL,
    grams            DOUBLE PRECISION NOT NULL CHECK (grams > 0 AND grams <= 5000),
    product_name     TEXT NOT NULL,
    product_brand    TEXT,
    energy_kcal      DOUBLE PRECISION NOT NULL,
    protein_g        DOUBLE PRECISION NOT NULL,
    fat_g            DOUBLE PRECISION NOT NULL,
    carbs_g          DOUBLE PRECISION NOT NULL,
    saturated_fat_g  DOUBLE PRECISION,
    sugars_g         DOUBLE PRECISION,
    fiber_g          DOUBLE PRECISION,
    salt_g           DOUBLE PRECISION,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX diary_entries_user_date_idx ON diary_entries (user_id, date);
CREATE INDEX diary_entries_user_product_idx ON diary_entries (user_id, product_id, created_at DESC);

CREATE TABLE body_profiles (
    user_id           UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    sex               TEXT NOT NULL CHECK (sex IN ('male', 'female')),
    height_cm         DOUBLE PRECISION NOT NULL CHECK (height_cm BETWEEN 100 AND 250),
    activity          TEXT NOT NULL CHECK (activity IN ('sedentary', 'light', 'moderate', 'active', 'very_active')),
    goal              TEXT NOT NULL CHECK (goal IN ('lose', 'maintain', 'gain')),
    pace_kg_per_week  DOUBLE PRECISION NOT NULL CHECK (pace_kg_per_week BETWEEN 0 AND 1),
    energy_kcal       DOUBLE PRECISION CHECK (energy_kcal BETWEEN 800 AND 6000),
    protein_g         DOUBLE PRECISION CHECK (protein_g BETWEEN 0 AND 500),
    fat_g             DOUBLE PRECISION CHECK (fat_g BETWEEN 0 AND 400),
    carbs_g           DOUBLE PRECISION CHECK (carbs_g BETWEEN 0 AND 1000),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE body_weights (
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    date        DATE NOT NULL,
    weight_kg   DOUBLE PRECISION NOT NULL CHECK (weight_kg BETWEEN 25 AND 400),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, date)
);
