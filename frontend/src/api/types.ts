// SPDX-License-Identifier: AGPL-3.0-or-later
export interface UserInfo {
  id: string;
  email: string;
  display_name: string;
  role: string;
  /** False on a server that mails confirmation links, until the link is used (revision 7). */
  email_verified: boolean;
  /**
   * False until the user turns it on, for every account. While false the
   * app must compute and show nothing derived from listening history —
   * not compute it and hide it. Nothing is stored server-side either
   * way; see docs/podcast-recommendations-plan.md §0.
   *
   * Optional on the wire so a client built against an older server (or a
   * cached login response) reads as "off" rather than crashing.
   */
  recommendations_enabled?: boolean;
}

export interface LoginResponse {
  token: string;
  /** Single-use and rotating — see `store/authStore.ts`. */
  refresh_token: string;
  user: UserInfo;
}

/** `private` (the unmarked default) or `family` (shared with the household). */
export type Visibility = "private" | "family";

export interface PodcastFeed {
  id: string;
  feed_url: string;
  source_type: string;
  title: string;
  description: string | null;
  author: string | null;
  link: string | null;
  language: string | null;
  image_url: string | null;
  last_refreshed_at: string | null;
  /** Lowercased, from the Podcast Index catalogue or the feed's own
      iTunes tags. Empty is normal — a feed followed while the metadata
      service was unreachable has none until the backfill reaches it. */
  categories: string[];
  /** The language subtag (`en`). Group and filter on this, never on
      `language`, which is whatever the publisher wrote — the wild
      contains 112 spellings of English. */
  language_base: string | null;
  visibility: Visibility;
  /** False for family-shared items owned by someone else. */
  is_owner: boolean;
  /** The server stores every new episode itself. Absent from older servers. */
  auto_store?: boolean;
  /** Some episode publishes a transcript, so episodes can be translated. Absent from older servers. */
  has_transcripts?: boolean;
}

export interface PodcastEpisode {
  id: string;
  feed_id: string;
  guid: string;
  title: string;
  description: string | null;
  published_at: string | null;
  duration_secs: number | null;
  episode_number: number | null;
  audio_url: string | null;
  image_url: string | null;
  has_local: boolean;
  progress_secs: number | null;
  completed: boolean;
  /** True when the feed published a transcript for this episode — the gate for
   * offering translation (see docs/podcast-translation-plan.md; no speech-to-text). */
  has_transcript: boolean;
}

export interface PodcastTranslationQuote {
  char_count: number;
  translation_cost_cents: number;
  tts_cost_cents: number;
  quoted_price_cents: number;
  currency: string;
  notice: string;
}

export type PodcastTranslationStatus = "translating" | "narrating" | "assembling" | "complete" | "failed";

export interface PodcastEpisodeTranslation {
  id: string;
  episode_id: string;
  source_language: string;
  target_language: string;
  voice_profile_id: string;
  status: PodcastTranslationStatus;
  char_count: number;
  quoted_price_cents: number;
  charged_price_cents: number | null;
  currency: string;
  error: string | null;
  stream_url: string | null;
  notice: string;
  /** Length of the finished audio. */
  duration_secs?: number | null;
}

/** A translation with the episode and show it belongs to (`GET /podcast-translate/recent`). */
export interface RecentPodcastTranslation extends PodcastEpisodeTranslation {
  episode_title: string;
  feed_id: string;
  feed_title: string;
  image_url: string | null;
}

export interface PodcastSearchResult {
  title: string;
  feed_url: string;
  link: string | null;
  description: string | null;
  author: string | null;
  image_url: string | null;
  language: string | null;
  episode_count: number | null;
  categories: string[];
}

/** One category of the catalogue, from `/podcasts/discover/categories`. */
export interface PodcastCategory {
  category: string;
  feed_count: number;
  /** Feeds that published within the last year. Show this one —
      `feed_count` is mostly shows that have ended. */
  active_count: number;
}

export interface AudioBook {
  id: string;
  title: string;
  author: string | null;
  narrator: string | null;
  description: string | null;
  cover_url: string | null;
  total_duration_secs: number | null;
  source_url: string | null;
  /** Non-null once the book has been matched through the identify flow. */
  google_books_volume_id: string | null;
  isbn: string | null;
  publisher: string | null;
  published_year: number | null;
  visibility: Visibility;
  is_owner: boolean;
  created_at: string;
  updated_at: string;
}

export interface AudioBookFile {
  id: string;
  book_id: string;
  position: number;
  title: string | null;
  duration_secs: number | null;
  audio_object_id: string;
}

export interface AudioBookChapter {
  id: string;
  book_id: string;
  file_id: string | null;
  position: number;
  title: string;
  start_time_secs: number;
}

// ── Library ───────────────────────────────────────────────────────────────

export type ContinueItem =
  | {
      kind: "Episode";
      episode_id: string;
      feed_id: string;
      episode_title: string;
      feed_title: string;
      position_secs: number;
      duration_secs: number | null;
      updated_at: string;
    }
  | {
      kind: "Book";
      book_id: string;
      book_title: string;
      author: string | null;
      position_secs: number;
      total_duration_secs: number | null;
      updated_at: string;
    };

export type SearchResult =
  | { kind: "Feed"; id: string; title: string; author: string | null; description: string | null }
  | { kind: "Episode"; id: string; feed_id: string; title: string; feed_title: string | null; published_at: string | null }
  | { kind: "Book"; id: string; title: string; author: string | null }
  | { kind: "Track"; id: string; title: string; artist: string | null };

// ── Music ─────────────────────────────────────────────────────────────────

export interface MusicTrack {
  id: string;
  title: string;
  artist: string | null;
  album: string | null;
  /** The album's credited artist, when the file tags one; groups albums ("Various Artists"). */
  album_artist?: string | null;
  genre: string | null;
  track_number: number | null;
  duration_secs: number | null;
  cover_url: string | null;
  musicbrainz_recording_id?: string | null;
  /** From `media_objects`; both nullable, and `sha256` can lag behind an
   *  upload while the checksum job catches up. */
  size_bytes?: number | null;
  sha256?: string | null;
  visibility: Visibility;
  is_owner: boolean;
  created_at: string;
  updated_at: string;
}

export interface ArtistSummary {
  artist: string;
  album_count: number;
  track_count: number;
}

export interface AlbumSummary {
  artist: string;
  album: string;
  track_count: number;
  duration_secs: number | null;
  cover_url: string | null;
}

export interface GenreSummary {
  genre: string;
  track_count: number;
}

export interface MusicPlaylist {
  id: string;
  name: string;
  description: string | null;
  cover_url: string | null;
  track_count: number;
  visibility: Visibility;
  is_owner: boolean;
  created_at: string;
  updated_at: string;
  /** When the smart-playlist generator made it; absent for an ordinary playlist. */
  generated_at?: string | null;
  /** When the owner chose to keep a generated playlist. */
  kept_at?: string | null;
}

export interface MusicPlaylistTrack {
  entry_id: string;
  position: number;
  track: MusicTrack;
}

// ── Stats ─────────────────────────────────────────────────────────────────

export interface HistoryEntry {
  id: string;
  media_kind: "audiobook" | "podcast" | "music" | string;
  item_id: string;
  part_id: string | null;
  title: string | null;
  started_at: string;
  ended_at: string;
  seconds: number;
  device_kind: string;
  source: "reported" | "derived";
}

export interface KindTotal {
  media_kind: "audiobook" | "podcast" | "music" | string;
  seconds: number;
  sessions: number;
}

export interface DayTotal {
  /** YYYY-MM-DD in the timezone the request asked for. */
  day: string;
  seconds: number;
}

export interface DayKindTotal {
  /** YYYY-MM-DD in the timezone the request asked for. */
  day: string;
  media_kind: "audiobook" | "podcast" | "music" | string;
  seconds: number;
}

export interface TopItem {
  media_kind: string;
  item_id: string;
  /** Null when the item has since been deleted. */
  title: string | null;
  seconds: number;
  sessions: number;
}

export interface StatsSummary {
  range: StatsRange;
  total_seconds: number;
  by_kind: KindTotal[];
  by_day: DayTotal[];
  /** `by_day` split by kind. Absent from older servers. */
  by_day_kind?: DayKindTotal[];
  top_items: TopItem[];
  streak_days: number;
  /** Lifetime count of finished audiobooks — not affected by the range. */
  completed_items: number;
}

export type StatsRange = "7d" | "30d" | "90d" | "365d" | "all";
export type StatsVisibility = "private" | "family_admin";

export interface FamilyStatsEntry {
  user_id: string;
  display_name: string;
  display_label: string | null;
  /** Absent when this member keeps their statistics private. */
  total_seconds?: number;
  by_kind?: KindTotal[];
  hidden: boolean;
}

export interface Notification {
  id: string;
  kind: string;
  title: string;
  body: string | null;
  data: Record<string, unknown> | null;
  created_at: string;
}

// ── Audiobook Authors, Collections, Sharing ──────────────────────────────

export interface AudiobookAuthor {
  id: string;
  name: string;
  sort_name: string | null;
  bio: string | null;
  image_url: string | null;
  book_count: number;
  created_at: string;
}

export interface BookAuthor {
  author_id: string;
  author_name: string;
  role: string;
}

export interface AudiobookTag {
  id: string;
  name: string;
}

export interface AudiobookCollection {
  id: string;
  name: string;
  description: string | null;
  cover_url: string | null;
  is_public: boolean;
  book_count: number;
  created_at: string;
  updated_at: string;
}

export interface AudiobookSeries {
  id: string;
  name: string;
  description: string | null;
  books: SeriesBookEntry[];
  created_at: string;
}

export interface SeriesBookEntry {
  book_id: string;
  book_title: string;
  position: number;
}

export interface CollectionBookEntry {
  id: string;
  title: string;
  author: string | null;
  narrator: string | null;
  cover_url: string | null;
  total_duration_secs: number | null;
  created_at: string;
}

// ── Audiobook Generation ──────────────────────────────────────────────────

export interface Language {
  code: string;
  label: string;
}

export interface VoiceProfile {
  id: string;
  display_name: string;
  language: string;
  gender: "male" | "female" | "neutral";
  preview_url: string | null;
  cost_per_million_chars_cents: number;
}

export interface GenerationQuote {
  char_count: number;
  translation_cost_cents: number;
  tts_cost_cents: number;
  internal_cost_cents: number;
  quoted_price_cents: number;
  currency: string;
}

export type GenerationStage =
  | "extracting"
  | "translating"
  | "preprocessing"
  | "narrating"
  | "assembling"
  | "complete"
  | "failed";

export interface ChapterProgress {
  id: string;
  title: string;
  status: "pending" | "in_progress" | "complete" | "failed";
  blocks_done: number;
  blocks_total: number;
}

export interface GenerationJobStatus {
  id: string;
  title: string;
  /** The ordered stages this specific job runs through (translation is omitted when not needed). */
  stages: GenerationStage[];
  stage: GenerationStage;
  error: string | null;
  chapters: ChapterProgress[];
  /** Whole-book download. Null for a `multi_file` job, which has no single
   *  file to hand over — not an error, and not a reason to hide the rest of
   *  the completion state. */
  download_url: string | null;
  /** The published book, once assembly has run. Both output modes publish
   *  one, so this is the deep-link that always works. */
  book_id: string | null;
  quoted_price_cents: number;
  /** Running total of real API cost incurred so far. audio2 is self-hosted — this is cost visibility, not a charge. */
  actual_cost_cents: number | null;
  /** Set once the job reaches "complete": min(actual_cost_cents * markup, quoted_price_cents). */
  charged_price_cents: number | null;
  currency: string;
}

/** A row from `GET /library/private`: one flat list across all three kinds. */
export interface PrivateItem {
  kind: "audiobook" | "podcast" | "music";
  id: string;
  title: string;
  subtitle: string | null;
  cover_url?: string | null;
  /** Songs only. */
  album?: string | null;
}
