-- Synthetic legacy SQLite fixture. No runtime dependency on Git or local reference files.
-- Source commit: 51319b733cc7c6b01ff39a3a627af95c9da69620
-- src/core/store.rs SHA-256 230c1960a1b5cc6bf0790ea960b3c5cab3e1631078aa1476538562a94b84b124
PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS retries(task TEXT NOT NULL,stage TEXT NOT NULL,count INTEGER NOT NULL,next_at INTEGER NOT NULL,PRIMARY KEY(task,stage));
            CREATE TABLE IF NOT EXISTS meetings(id TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, session TEXT NOT NULL REFERENCES meetings(id), attempt INTEGER NOT NULL,
              input_hash TEXT NOT NULL, config_hash TEXT NOT NULL, document TEXT NOT NULL, stage TEXT NOT NULL, status TEXT NOT NULL,
              lease TEXT, expires INTEGER, directory TEXT NOT NULL, config TEXT NOT NULL, segments TEXT NOT NULL,
              UNIQUE(session, attempt));
            CREATE TABLE IF NOT EXISTS commits(task TEXT NOT NULL REFERENCES tasks(id), stage TEXT NOT NULL, lease TEXT NOT NULL,
              staging TEXT NOT NULL, destination TEXT NOT NULL, hash TEXT NOT NULL, metadata TEXT, done INTEGER NOT NULL DEFAULT 0,
              PRIMARY KEY(task,stage));
-- src/core/store/segments.rs SHA-256 9ddd36ab8393704668e24d4ba101cd97cf8901a44fa1a7cc67b6417c3ee92a0b
CREATE TABLE IF NOT EXISTS segmented_sessions(session TEXT PRIMARY KEY REFERENCES meetings(id),parent TEXT NOT NULL REFERENCES tasks(id),sealed INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS segment_jobs(session TEXT NOT NULL REFERENCES segmented_sessions(session),sequence INTEGER NOT NULL,offset_frames INTEGER NOT NULL,frames INTEGER NOT NULL,audio TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'queued',lease TEXT,expires INTEGER,retries INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,error TEXT,staging TEXT,destination TEXT,hash TEXT,PRIMARY KEY(session,sequence));
        CREATE TABLE IF NOT EXISTS segment_activity(lease TEXT PRIMARY KEY,document TEXT NOT NULL);
-- src/core/store/archives.rs SHA-256 78f4e42f3c7e0661d13d3c4b2a0f1f42ea719a052bb93c4a040d320201e5377e
CREATE TABLE IF NOT EXISTS segment_archives(session TEXT NOT NULL,sequence INTEGER NOT NULL,status TEXT NOT NULL DEFAULT 'queued',lease TEXT,expires INTEGER,receipt TEXT,retries INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,error TEXT,cleaned INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(session,sequence),FOREIGN KEY(session,sequence) REFERENCES segment_jobs(session,sequence));INSERT OR IGNORE INTO segment_archives(session,sequence) SELECT session,sequence FROM segment_jobs;
ALTER TABLE segment_archives ADD COLUMN bitrate INTEGER NOT NULL DEFAULT 48000;
