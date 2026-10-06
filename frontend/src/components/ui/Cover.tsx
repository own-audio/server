// SPDX-License-Identifier: AGPL-3.0-or-later
import { BookIcon, MusicIcon, PodcastIcon } from "./CloudIcon";
import AuthImage from "../AuthImage";
import { mediaUrl } from "../../api/client";
import { cn } from "../../lib/cn";

export type MediaKind = "audiobook" | "podcast" | "music";

const tint: Record<MediaKind, string> = {
  audiobook: "cloud-tint-book text-book",
  podcast: "cloud-tint-podcast text-podcast",
  music: "cloud-tint-music text-music",
};

const Icon = ({ kind, className }: { kind: MediaKind; className?: string }) =>
  kind === "audiobook" ? <BookIcon className={className} /> : kind === "podcast" ? <PodcastIcon className={className} /> : <MusicIcon className={className} />;

interface CoverProps {
  kind: MediaKind;
  src?: string | null;
  alt: string;
  /** Audiobooks are 2:3 portrait; podcasts and music are square. */
  aspect?: "portrait" | "square";
  /** Podcast art is public; audiobook and music covers need the auth header. */
  auth?: boolean;
  /** A person's photo (an artist) is a circle; releases keep the cover radius. */
  round?: boolean;
  className?: string;
}

export function Cover({ kind, src, alt, aspect, auth = kind !== "podcast", round, className }: CoverProps) {
  const ratio = (aspect ?? (kind === "audiobook" ? "portrait" : "square")) === "portrait" ? "aspect-[2/3]" : "aspect-square";
  const placeholder = (
    <div className={cn("flex h-full w-full items-center justify-center", tint[kind])} aria-hidden="true">
      <Icon kind={kind} className="h-[28%] w-[28%] opacity-70" />
    </div>
  );
  return (
    <div className={cn("relative overflow-hidden bg-bg-alt", round ? "rounded-full" : "rounded-cover", ratio, className)}>
      {src ? (
        auth ? (
          // The placeholder stands in while it loads and if it fails, so a
          // cover that cannot be fetched looks deliberate rather than broken.
          <AuthImage src={src} alt={alt} className="h-full w-full object-cover" fallback={placeholder} />
        ) : (
          <img src={mediaUrl(src)} alt={alt} className="h-full w-full object-cover" loading="lazy" />
        )
      ) : (
        placeholder
      )}
    </div>
  );
}
