// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
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
  // An authenticated cover is asked for only while it is near the screen: a long
  // list would otherwise ask for every cover in it, and a fast scroll for every
  // cover it passed. One that scrolls away before its turn is never fetched; one
  // already loaded comes back from the cache (lib/authImages.ts).
  const box = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);
  useEffect(() => {
    const el = box.current;
    if (!el || !src || !auth) return;
    const io = new IntersectionObserver(([e]) => setNear(e.isIntersecting), { rootMargin: "400px" });
    io.observe(el);
    return () => io.disconnect();
  }, [src, auth]);

  return (
    <div ref={box} className={cn("relative overflow-hidden bg-bg-alt", round ? "rounded-full" : "rounded-cover", ratio, className)}>
      {src && (!auth || near) ? (
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
