/// Default retention and size budget for one storage column family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionDefault {
    pub cf: &'static str,
    pub ttl: RetentionTtl,
    pub soft_cap_mb: u64,
    pub hard_cap_mb: u64,
}

/// Default TTL policy for a storage column family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionTtl {
    None,
    Hours(u64),
    Days(u64),
    LruOnly,
}

/// PRD §4/§6 storage retention defaults.
pub const DEFAULTS: [RetentionDefault; 12] = [
    RetentionDefault {
        cf: "CF_EVENTS",
        ttl: RetentionTtl::Hours(24),
        soft_cap_mb: 2048,
        hard_cap_mb: 4096,
    },
    RetentionDefault {
        cf: "CF_OBSERVATIONS",
        ttl: RetentionTtl::Hours(6),
        soft_cap_mb: 500,
        hard_cap_mb: 1000,
    },
    RetentionDefault {
        cf: "CF_PROFILES",
        ttl: RetentionTtl::None,
        soft_cap_mb: 20,
        hard_cap_mb: 50,
    },
    RetentionDefault {
        cf: "CF_MODEL_CACHE",
        ttl: RetentionTtl::LruOnly,
        soft_cap_mb: 1024,
        hard_cap_mb: 2048,
    },
    RetentionDefault {
        cf: "CF_SESSIONS",
        ttl: RetentionTtl::Days(30),
        soft_cap_mb: 50,
        hard_cap_mb: 100,
    },
    RetentionDefault {
        cf: "CF_REFLEX_AUDIT",
        ttl: RetentionTtl::Days(7),
        soft_cap_mb: 200,
        hard_cap_mb: 500,
    },
    RetentionDefault {
        cf: "CF_OCR_CACHE",
        ttl: RetentionTtl::Hours(1),
        soft_cap_mb: 50,
        hard_cap_mb: 100,
    },
    RetentionDefault {
        cf: "CF_TELEMETRY",
        ttl: RetentionTtl::Hours(6),
        soft_cap_mb: 100,
        hard_cap_mb: 200,
    },
    RetentionDefault {
        cf: "CF_ACTION_LOG",
        ttl: RetentionTtl::Hours(24),
        soft_cap_mb: 200,
        hard_cap_mb: 500,
    },
    RetentionDefault {
        cf: "CF_PROCESS_HISTORY",
        ttl: RetentionTtl::Hours(6),
        soft_cap_mb: 20,
        hard_cap_mb: 50,
    },
    RetentionDefault {
        cf: "CF_KV",
        ttl: RetentionTtl::None,
        soft_cap_mb: 10,
        hard_cap_mb: 50,
    },
    // ADR 2026-06-11-timeline-data-model: long-retention operator activity
    // timeline; TTL eviction additionally relies on periodic compaction so
    // cold SST files still pass the TTL filter (see storage cf_options).
    RetentionDefault {
        cf: "CF_TIMELINE",
        ttl: RetentionTtl::Days(90),
        soft_cap_mb: 4096,
        hard_cap_mb: 8192,
    },
];
