// SPDX-License-Identifier: AGPL-3.0-or-later
// A MusicBrainz match's cover, small: the release's front from Cover Art
// Archive, else its release group's, else a plain square. People tell
// editions and recordings apart by their cover first.
import { useState } from "react";
import { Music } from "lucide-react";
import { cn } from "../lib/cn";

function coverArchiveFronts(release?: string | null, group?: string | null, size = 250): string[] {
  return [release && `release/${release}`, group && `release-group/${group}`]
    .filter((p): p is string => !!p)
    .map((p) => `https://coverartarchive.org/${p}/front-${size}`);
}

export function CoverArchiveThumb({ release, group, size = 40, className }: {
  release?: string | null;
  group?: string | null;
  size?: number;
  className?: string;
}) {
  const urls = coverArchiveFronts(release, group);
  const [i, setI] = useState(0);
  const box = { width: size, height: size };
  if (i >= urls.length) {
    return (
      <span style={box} className={cn("inline-flex shrink-0 items-center justify-center rounded bg-bg-alt text-muted", className)} aria-hidden>
        <Music className="h-4 w-4" />
      </span>
    );
  }
  return (
    <img
      key={urls[i]}
      src={urls[i]}
      alt=""
      loading="lazy"
      style={box}
      className={cn("shrink-0 rounded bg-bg-alt object-cover", className)}
      onError={() => setI((n) => n + 1)}
    />
  );
}
