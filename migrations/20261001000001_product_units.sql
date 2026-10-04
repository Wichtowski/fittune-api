-- Drinks are labelled per 100 ml: every product has a unit, values are per 100 of it, and
-- amounts and servings are in that unit

ALTER TABLE food_products ADD COLUMN unit TEXT NOT NULL DEFAULT 'g' CHECK (unit IN ('g', 'ml'));
ALTER TABLE food_products RENAME COLUMN serving_g TO serving_amount;

ALTER TABLE off_products ADD COLUMN unit TEXT NOT NULL DEFAULT 'g' CHECK (unit IN ('g', 'ml'));
ALTER TABLE off_products RENAME COLUMN serving_g TO serving_amount;

-- Entries keep the unit they were logged in, like the rest of their product snapshot
ALTER TABLE diary_entries RENAME COLUMN grams TO amount;
ALTER TABLE diary_entries ADD COLUMN unit TEXT NOT NULL DEFAULT 'g' CHECK (unit IN ('g', 'ml'));
