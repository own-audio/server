// SPDX-License-Identifier: AGPL-3.0-or-later
import axios from "axios";
import api from "./client";

/* `GET /server` — what this server is and what it offers. The contract is
   own-audio-foss/docs/API_COMPATIBILITY.md §3: a key is true only when the
   feature is fully configured, unknown keys are ignored, missing keys are
   false. The console hides what is false; it never guesses from `edition`. */

export interface ServerFeatures {
  registration_open: boolean;
  auth: { local: boolean; google: boolean; apple: boolean; microsoft: boolean };
  uploads: { presigned: boolean; multipart_max_bytes: number | null };
  music_identify: boolean;
  podcast_discovery: boolean;
  file_sync: boolean;
  library_folders: boolean;
  subsonic: boolean;
  mail: boolean;
  billing: boolean;
  payments: boolean;
  narration: boolean;
  translation: boolean;
}

export interface ServerInfo {
  name: string;
  edition: string;
  version: string;
  api: { version: number; revision: number };
  features: ServerFeatures;
  deprecations: unknown[];
  /** Only on a public demo: the shared account visitors sign in with. */
  demo?: { email: string; password: string };
}

export const NO_FEATURES: ServerFeatures = {
  registration_open: false,
  auth: { local: true, google: false, apple: false, microsoft: false },
  uploads: { presigned: true, multipart_max_bytes: null },
  music_identify: false,
  podcast_discovery: false,
  file_sync: true,
  library_folders: false,
  subsonic: true,
  mail: false,
  billing: false,
  payments: false,
  narration: false,
  translation: false,
};

/* A server older than `GET /server` answers 404. The hosted console is
   deployed ahead of the hosted API (Pages on push, the API via canary and a
   manual promote), so for a while this console can meet such a server — and
   it must keep showing what it always showed there. Remove once production
   reports api.revision ≥ 1; native clients treat 404 as "nothing optional"
   instead, per the policy. */
const LEGACY_HOSTED: ServerInfo = {
  name: "own.audio",
  edition: "legacy",
  version: "",
  api: { version: 1, revision: 0 },
  features: { ...NO_FEATURES, billing: true, narration: true, translation: true },
  deprecations: [],
};

export async function getServerInfo(): Promise<ServerInfo> {
  try {
    const { data } = await api.get<ServerInfo>("/server");
    return { ...data, features: { ...NO_FEATURES, ...data.features } };
  } catch (e) {
    if (axios.isAxiosError(e) && e.response?.status === 404) return LEGACY_HOSTED;
    throw e;
  }
}
