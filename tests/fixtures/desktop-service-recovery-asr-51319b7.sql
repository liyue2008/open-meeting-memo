-- Synthetic legacy SQLite fixture. No runtime dependency on Git or local reference files.
-- Source commit: 51319b733cc7c6b01ff39a3a627af95c9da69620
-- src/core/store/asr_measurements.rs:76 SHA-256 b86039b50e3e72c1b021984ed6a2cfbceb61e256336c456a9f4e19657a4d2873
-- Normal caller: src/adapters/asr/mod.rs:389-393 (same commit).
-- Two sequential autocommit CREATEs precede unchecked_transaction(): interruption can
-- leave only asr_measurements, never only asr_measurement_totals. No explicit indexes
-- or triggers. SQLite creates only the two primary-key autoindexes.
CREATE TABLE IF NOT EXISTS asr_measurements(task TEXT NOT NULL,slot INTEGER NOT NULL,ordinal INTEGER NOT NULL,document TEXT NOT NULL CHECK(length(CAST(document AS BLOB))<=1024),PRIMARY KEY(task,slot)); CREATE TABLE IF NOT EXISTS asr_measurement_totals(task TEXT PRIMARY KEY,document TEXT NOT NULL);
