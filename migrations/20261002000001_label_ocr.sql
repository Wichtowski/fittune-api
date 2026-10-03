CREATE TABLE app_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    ocr_model TEXT NOT NULL DEFAULT 'gpt-6-luna',
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO app_settings (singleton) VALUES (TRUE);

CREATE TABLE ocr_ai_daily (
    day DATE NOT NULL,
    subject TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    PRIMARY KEY (day, subject)
);
