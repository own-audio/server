-- Migration 0001: enable extensions needed by all subsequent migrations
CREATE EXTENSION IF NOT EXISTS "pgcrypto"; -- gen_random_uuid()
