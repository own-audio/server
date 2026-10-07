// SPDX-License-Identifier: AGPL-3.0-or-later
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import AppShell from "./components/shell/AppShell";
import ProtectedRoute from "./components/ProtectedRoute";
import FeatureGate from "./components/FeatureGate";
import NotFound from "./pages/NotFound";
import AuthPage from "./pages/auth/AuthPage";
import SectionTheme from "./components/shell/SectionTheme";
import { Suspense, type ReactNode } from "react";
import { lazyPage } from "./lib/lazyPage";

const SetupWizard = lazyPage(() => import("./pages/setup/SetupWizard"));
const HomePage = lazyPage(() => import("./pages/home/HomePage"));
const PodcastsPage = lazyPage(() => import("./pages/podcasts/PodcastsPage"));
const AudiobooksPage = lazyPage(() => import("./pages/audiobooks/AudiobooksPage"));
const OrganizeRedirect = lazyPage(() => import("./pages/audiobooks/Organize").then((m) => ({ default: m.OrganizeRedirect })));
const MusicPage = lazyPage(() => import("./pages/music/MusicPage"));
const DuplicatesPage = lazyPage(() => import("./pages/music/MusicTools"));
const SettingsPage = lazyPage(() => import("./pages/settings/SettingsPage"));
const PlaybackSettingsPage = lazyPage(() => import("./pages/settings/PlaybackSettingsPage"));
const PrivatePage = lazyPage(() => import("./pages/library/PrivatePage"));
const StatsPage = lazyPage(() => import("./pages/stats/StatsPage"));
const FamilyPage = lazyPage(() => import("./pages/family/FamilyPage"));
const BillingPage = lazyPage(() => import("./pages/billing/BillingPage"));
const TrashPage = lazyPage(() => import("./pages/trash/TrashPage"));
const JoinPage = lazyPage(() => import("./pages/join/JoinPage"));
const LinkPage = lazyPage(() => import("./pages/link/LinkPage"));
const PlayPage = lazyPage(() => import("./pages/play/PlayPage"));
const TranslatePage = lazyPage(() => import("./pages/podcasts/TranslatePage"));
const GenerateWizardPage = lazyPage(() => import("./pages/generate/GenerateWizardPage"));
const GenerationStatusPage = lazyPage(() => import("./pages/generate/GenerationStatusPage"));

interface AppProps {
  /** Additional route elements to render inside the protected layout */
  extraRoutes?: ReactNode;
}

/* Routes follow the sidebar. Inside a section, selecting an item is a URL
   change but not a new screen — the section page renders both columns, so a
   deep link opens the list with that item selected. */
export default function App({ extraRoutes }: AppProps = {}) {
  return (
    <BrowserRouter>
      {/* Pages outside the shell; pages inside it suspend within AppShell, so the sidebar stays put. */}
      <Suspense fallback={null}>
      <Routes>
        <Route path="/auth/login" element={<AuthPage />} />
        <Route path="/setup" element={<SetupWizard />} />
        {/* Public: an invite link has to work signed out. */}
        <Route path="/join/:code" element={<JoinPage />} />

        {/* Approving a TV's code is an account action, so it signs you in first
            and comes back here — with whatever provider you use, which is the
            point: tvOS has no Google sign-in of its own. */}
        <Route
          path="/link"
          element={
            <ProtectedRoute>
              <LinkPage />
            </ProtectedRoute>
          }
        />

        {/* A playlist's own Home Screen icon opens here: no sidebar, just the
            playlist, offline. It handles its own sign-in — Safari only shows
            the add-to-Home-Screen step, and a new icon signs itself in. */}
        <Route path="/play/:playlistId" element={<PlayPage />} />

        <Route
          path="/"
          element={
            <ProtectedRoute>
              <AppShell />
            </ProtectedRoute>
          }
        >
          <Route index element={<HomePage />} />
          <Route path="library" element={<Navigate to="/" replace />} />

          <Route path="podcasts" element={<PodcastsPage />} />
          <Route path="podcasts/add" element={<PodcastsPage />} />
          <Route path="podcasts/:feedId" element={<PodcastsPage />} />

          <Route path="audiobooks" element={<AudiobooksPage />} />
          <Route path="audiobooks/upload" element={<AudiobooksPage />} />
          <Route path="audiobooks/:bookId" element={<AudiobooksPage />} />
          <Route path="audiobooks/series/:groupId" element={<AudiobooksPage />} />
          <Route path="audiobooks/collections/:groupId" element={<AudiobooksPage />} />
          <Route path="audiobooks/authors/:groupId" element={<AudiobooksPage />} />
          <Route path="audiobooks/organize" element={<OrganizeRedirect />} />
          <Route path="audiobooks/organize/:groupId" element={<OrganizeRedirect />} />

          <Route path="music" element={<MusicPage />} />
          <Route path="music/upload" element={<MusicPage />} />
          <Route path="music/artists/:artist" element={<MusicPage />} />
          <Route path="music/albums" element={<MusicPage />} />
          <Route path="music/albums/:albumKey" element={<MusicPage />} />
          <Route path="music/genres/:genre" element={<MusicPage />} />
          <Route path="music/playlists" element={<MusicPage />} />
          <Route path="music/playlists/:playlistId" element={<MusicPage />} />
          {/* Its own page, not a MusicPage mode, so it needs the section colour set here. */}
          <Route path="music/duplicates" element={<SectionTheme cloud="music"><DuplicatesPage /></SectionTheme>} />
          <Route path="music/downloads" element={<MusicPage />} />

          {/* Hosted-only features: on a server that reports them false these routes go home. */}
          <Route path="generate" element={<FeatureGate feature="narration"><GenerateWizardPage /></FeatureGate>} />
          <Route path="translate" element={<FeatureGate feature="translation"><TranslatePage /></FeatureGate>} />
          <Route path="generate/:jobId" element={<FeatureGate feature="narration"><GenerationStatusPage /></FeatureGate>} />
          <Route path="private" element={<PrivatePage />} />
          <Route path="stats" element={<StatsPage />} />
          <Route path="family" element={<FamilyPage />} />
          <Route path="billing" element={<FeatureGate feature="billing"><BillingPage /></FeatureGate>} />
          <Route path="trash" element={<TrashPage />} />
          <Route path="settings" element={<SettingsPage />} />
          <Route path="settings/playback" element={<PlaybackSettingsPage />} />
          {/* The instance console is its own app (admin/); this page duplicated it. */}
          <Route path="admin" element={<Navigate to="/" replace />} />
          {extraRoutes}
        </Route>

        <Route path="*" element={<NotFound />} />
      </Routes>
      </Suspense>
    </BrowserRouter>
  );
}
