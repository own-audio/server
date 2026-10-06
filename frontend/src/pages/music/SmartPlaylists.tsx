// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { CalendarPlus, Check, Heart, ListMusic, Loader2, Trash2, Play, Radio, RefreshCcw, Sparkles, Users, Wand2 } from "lucide-react";
import {
  createPlaylist,
  deletePlaylist,
  keepPlaylist,
  listGenres,
  listPlaylists,
  listSmartPlaylistPresets,
  parseIntent,
  resolveRule,
  type PlaylistRule,
  type ResolvedPlaylist,
} from "../../api/music";
import { playTracks } from "../../lib/play";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, EmptyState, IconButton, Input, MenuItem, Skeleton, toast } from "../../components/ui";
import { intlLocale, t as translate, useT, type PlainKey } from "../../i18n";
import { MediaRow } from "../../components/library/MediaCard";
import { MoreMenuTrigger } from "../../components/library/BrowseControls";
import { moveToTrash } from "../../lib/trash";

/**
 * Smart playlists: a saved query rather than a list of tracks.
 *
 * The presets do the work the feature actually exists for — a family library's
 * problem is not *recommend something unknown* but **resurface something
 * forgotten** — and four of the five need no acoustic data at all, so they
 * work before anything has been measured.
 *
 * There is deliberately no rule editor here. Ship the presets, see which get
 * used, and let that decide whether an editor is wanted at all.
 */

function formatDuration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return h > 0 ? translate("common.duration.hoursMinutes", { h, m }) : translate("common.duration.minutes", { m });
}

/**
 * Why a playlist came back shorter than asked for.
 *
 * The honest limit of this feature is the library, not the algorithm: a narrow
 * request over a small or half-measured collection returns what exists. Saying
 * so is the difference between a feature that looks broken and one that looks
 * truthful.
 */
function ShortResultNote({ result, asked }: { result: ResolvedPlaylist; asked: PlaylistRule }) {
  const { t } = useT();
  const wantedMinutes = asked.limit?.kind === "duration" ? asked.limit.minutes : null;
  const gotMinutes = Math.round(result.total_secs / 60);
  if (wantedMinutes === null || gotMinutes >= wantedMinutes - 5) return null;

  return (
    <p className="mt-2 text-xs text-muted">
      {t("music.smart.shortResult", { candidates: result.candidates, got: gotMinutes, wanted: wantedMinutes })}
    </p>
  );
}

/**
 * Playing a generated list saves it first, as an ordinary playlist marked as generated, and
 * opens it: from then on it is a playlist like any other — its own page, a cover taken from one
 * of its songs, resumable from the queue — and its bar asks whether to keep it.
 */
function usePlayGenerated() {
  const { t } = useT();
  const qc = useQueryClient();
  const navigate = useNavigate();
  return useMutation({
    mutationFn: async ({ name, result }: { name: string; result: ResolvedPlaylist }) => {
      const when = new Intl.DateTimeFormat(intlLocale(), { dateStyle: "medium", timeStyle: "short" }).format(new Date());
      const playlist = await createPlaylist({
        name,
        description: t("music.smart.savedDescription", { when }),
        generated: true,
        track_ids: result.tracks.map((tr) => tr.id),
      });
      return { playlist, result };
    },
    onSuccess: ({ playlist, result }) => {
      qc.invalidateQueries({ queryKey: ["playlists"] });
      navigate(`/music/playlists/${playlist.id}`);
      void playTracks(result.tracks, 0, { kind: "playlist", label: playlist.name, path: `/music/playlists/${playlist.id}` });
    },
    onError: (err) => toast.error(t("music.smart.playFailed"), apiErrorMessage(err, t("music.toast.tryAgain"))),
  });
}

function ResultActions({
  result,
  rule,
  saveName,
}: {
  result: ResolvedPlaylist;
  rule: PlaylistRule;
  /** The playlist's name once played: the sentence it was asked with, or the preset's name. */
  saveName: string;
}) {
  const { t } = useT();
  const playGenerated = usePlayGenerated();

  return (
    <div className="mt-3 rounded-card border border-border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm">
          {t("music.count.songsWithDuration", { count: result.tracks.length, duration: formatDuration(result.total_secs) })}
        </p>
        <Button
          size="sm"
          icon={<Play className="h-4 w-4 fill-current" />}
          loading={playGenerated.isPending}
          onClick={() => playGenerated.mutate({ name: saveName, result })}
        >
          {t("common.action.play")}
        </Button>
      </div>
      <ShortResultNote result={result} asked={rule} />
      <ul className="mt-2 space-y-0.5 text-sm text-muted">
        {result.tracks.slice(0, 5).map((track) => (
          <li key={track.id} className="truncate">
            {track.artist ? `${track.artist} — ` : ""}
            {track.title}
          </li>
        ))}
        {result.tracks.length > 5 && <li>{t("music.count.andMore", { count: result.tracks.length - 5 })}</li>}
      </ul>
    </div>
  );
}

/**
 * Ask in words.
 *
 * The server turns the sentence into a **rule**, never a tracklist — so what
 * comes back is inspectable, and nothing plays until asked. What leaves the
 * browser is the sentence and the library's genre names; never track titles,
 * never listening history.
 */
function AskBox() {
  const { t } = useT();
  const [text, setText] = useState("");
  const [rule, setRule] = useState<PlaylistRule | null>(null);

  const ask = useMutation({
    mutationFn: async (): Promise<{ rule: PlaylistRule; result: ResolvedPlaylist; sentence: string }> => {
      const sentence = text.trim();
      const parsed = await parseIntent(sentence);
      return { rule: parsed, result: await resolveRule(parsed), sentence };
    },
    onSuccess: ({ rule: r }) => setRule(r),
    onError: (err) =>
      toast.error(
        t("music.smart.askFailed"),
        apiErrorMessage(err, t("music.smart.askFailedHint"))
      ),
  });

  return (
    <div className="rounded-card border border-border p-4">
      <p className="flex items-center gap-2 text-sm font-medium">
        <Wand2 className="h-4 w-4" /> {t("music.smart.askTitle")}
      </p>
      <p className="mt-1 text-sm text-muted">{t("music.smart.askHint")}</p>

      <form
        className="mt-3 flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) ask.mutate();
        }}
      >
        <Input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={t("music.smart.askPlaceholder")}
          aria-label={t("music.smart.askLabel")}
        />
        <Button type="submit" disabled={!text.trim() || ask.isPending}>
          {t(ask.isPending ? "music.smart.thinking" : "music.smart.ask")}
        </Button>
      </form>

      {ask.data && rule && (
        <ResultActions
          result={ask.data.result}
          rule={rule}
          saveName={ask.data.sentence}
        />
      )}
    </div>
  );
}

/* The server sends English names (the apps show them as they come); the web names and explains
   the presets it knows, and falls back to the server's name for any it doesn't. */
const PRESET_TEXT: Record<string, { name: PlainKey; body: PlainKey; icon: ReactNode }> = {
  "forgotten-favourites": { name: "music.smart.preset.forgottenFavourites.name", body: "music.smart.preset.forgottenFavourites.body", icon: <Heart /> },
  "new-in-the-library": { name: "music.smart.preset.newInTheLibrary.name", body: "music.smart.preset.newInTheLibrary.body", icon: <CalendarPlus /> },
  "back-in-rotation": { name: "music.smart.preset.backInRotation.name", body: "music.smart.preset.backInRotation.body", icon: <RefreshCcw /> },
  "what-the-family-plays": { name: "music.smart.preset.whatTheFamilyPlays.name", body: "music.smart.preset.whatTheFamilyPlays.body", icon: <Users /> },
  "genre-station": { name: "music.smart.preset.genreStation.name", body: "music.smart.preset.genreStation.body", icon: <Radio /> },
};

function PresetCard({ slug, name: serverName, rule: presetRule }: { slug: string; name: string; rule: PlaylistRule }) {
  const { t } = useT();
  const text = PRESET_TEXT[slug];
  const name = text ? t(text.name) : serverName;
  // The genre station is a template: the server leaves the genre for the client to fill in.
  const isStation = slug === "genre-station";
  const [genre, setGenre] = useState("");
  const { data: genres = [] } = useQuery({ queryKey: ["music-genres"], queryFn: listGenres, enabled: isStation });
  const rule: PlaylistRule =
    isStation && genre ? { ...presetRule, filters: { ...presetRule.filters, genres: { include: [genre] } } } : presetRule;

  const saveName = isStation && genre ? t("music.smart.presetWithGenre", { name, genre }) : name;
  const playGenerated = usePlayGenerated();
  const run = useMutation({
    // The variable says whether to play once built (the play button: saved and opened as a
    // playlist) or only list it here (Preview).
    mutationFn: async (play: boolean) => ({ result: await resolveRule(rule), play }),
    onSuccess: ({ result, play }) => {
      if (play && result.tracks.length > 0) playGenerated.mutate({ name: saveName, result });
    },
    onError: (err) => toast.error(t("music.smart.buildFailed"), apiErrorMessage(err, t("music.toast.tryAgain"))),
  });

  return (
    <div className="rounded-card border border-border p-3" data-preset={slug}>
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-pill bg-accent/12 text-accent-text [&>svg]:h-5 [&>svg]:w-5">
          {text?.icon ?? <Sparkles />}
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{name}</p>
          {text && <p className="mt-0.5 text-xs text-muted">{t(text.body)}</p>}
          {isStation && (
            <select
              value={genre}
              onChange={(e) => { setGenre(e.target.value); run.reset(); }}
              aria-label={t("music.smart.genre")}
              className="mt-2 h-9 w-full max-w-xs rounded-pill border border-border bg-bg-alt px-3 text-sm pointer-coarse:h-10"
            >
              <option value="">{t("music.smart.allGenres")}</option>
              {genres.map((g) => <option key={g.genre} value={g.genre}>{g.genre}</option>)}
            </select>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <Button size="sm" variant="ghost" disabled={run.isPending} onClick={() => run.mutate(false)}>
            {t("music.smart.preview")}
          </Button>
          <IconButton tone="accent" label={t("music.smart.playPreset", { name })} disabled={run.isPending || playGenerated.isPending} onClick={() => run.mutate(true)}>
            {run.isPending || playGenerated.isPending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Play className="h-4 w-4 translate-x-px fill-current" />}
          </IconButton>
        </div>
      </div>

      {run.data &&
        (run.data.result.tracks.length === 0 ? (
          // Not an error, and worth wording as a fact about the library: the
          // preset works, there is just nothing that fits it yet.
          <p className="mt-2 text-xs text-muted">{t("music.smart.nothingFits")}</p>
        ) : (
          <ResultActions
            result={run.data.result}
            rule={rule}
            saveName={saveName}
          />
        ))}
    </div>
  );
}

/**
 * Playlists saved from the generator, newest first — ordinary playlists, also listed in
 * Playlists. Every play saves one, so they pile up: each has Keep and Move to Trash, and the
 * ones never kept can go in one step (with Undo, like any other move to the trash).
 */
function GeneratedPlaylists() {
  const { t } = useT();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { data: playlists = [] } = useQuery({ queryKey: ["playlists"], queryFn: listPlaylists });
  const generated = playlists
    .filter((p) => p.generated_at && p.is_owner)
    .sort((a, b) => (b.generated_at ?? "").localeCompare(a.generated_at ?? ""));
  const unkept = generated.filter((p) => !p.kept_at);
  const refresh = () => qc.invalidateQueries({ queryKey: ["playlists"] });

  const keep = useMutation({
    mutationFn: keepPlaylist,
    onSuccess: () => { refresh(); toast.success(t("music.smart.kept")); },
    onError: () => toast.error(t("common.error.generic")),
  });
  const trash = (ids: string[], title: string) =>
    moveToTrash({
      title,
      remove: async (batch) => { for (const id of ids) await deletePlaylist(id, batch); },
      onChanged: refresh,
    }).catch(() => toast.error(t("common.error.generic")));

  if (generated.length === 0) return null;
  return (
    <section>
      <div className="mb-1 flex flex-wrap items-center justify-between gap-2">
        <h2 className="flex items-center gap-2 text-sm font-semibold uppercase tracking-wide text-muted">
          <ListMusic className="h-4 w-4" /> {t("music.smart.generated")}
        </h2>
        {unkept.length > 1 && (
          <Button size="sm" variant="ghost" icon={<Trash2 className="h-4 w-4" />} onClick={() => void trash(unkept.map((p) => p.id), t("music.smart.unkeptCount", { count: unkept.length }))}>
            {t("music.smart.clearUnkept", { count: unkept.length })}
          </Button>
        )}
      </div>
      <div className="-mx-2">
        {generated.map((p) => (
          <MediaRow
            key={p.id}
            kind="music"
            title={p.name}
            subtitle={p.kept_at ? p.description : [p.description, t("music.smart.notKept")].filter(Boolean).join(" · ")}
            cover={p.cover_url}
            trailing={t("music.count.songs", { count: p.track_count })}
            onClick={() => navigate(`/music/playlists/${p.id}`)}
            menu={
              <MoreMenuTrigger>
                {!p.kept_at && <MenuItem icon={<Check />} onSelect={() => keep.mutate(p.id)}>{t("music.smart.keep")}</MenuItem>}
                <MenuItem icon={<Trash2 />} destructive onSelect={() => void trash([p.id], p.name)}>{t("common.action.moveToTrash")}</MenuItem>
              </MoreMenuTrigger>
            }
          />
        ))}
      </div>
    </section>
  );
}

export default function SmartPlaylists() {
  const { t } = useT();
  const { data: presets, isLoading } = useQuery({
    queryKey: ["smart-playlist-presets"],
    queryFn: listSmartPlaylistPresets,
    staleTime: Infinity,
  });

  return (
    <div className="space-y-4 p-3">
      <AskBox />
      <GeneratedPlaylists />

      <section>
        <h2 className="mb-2 flex items-center gap-2 text-sm font-semibold uppercase tracking-wide text-muted">
          <Sparkles className="h-4 w-4" /> {t("music.smart.readyMade")}
        </h2>

        {isLoading ? (
          <div className="space-y-2">
            <Skeleton className="h-16" />
            <Skeleton className="h-16" />
          </div>
        ) : presets && presets.length > 0 ? (
          <div className="space-y-2">
            {presets.map((p) => (
              <PresetCard key={p.slug} slug={p.slug} name={p.name} rule={p.rule} />
            ))}
          </div>
        ) : (
          <EmptyState title={t("music.smart.noPresets")} description={t("music.smart.noPresetsBody")} />
        )}
      </section>
    </div>
  );
}
