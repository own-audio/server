// SPDX-License-Identifier: AGPL-3.0-or-later
import { forwardRef, type SVGProps } from "react";

/* The three clouds' glyphs — the same closed book, microphone and beamed notes
   as the Book, Podcast and Music app icons, so a section looks the same in the
   sidebar, a cover placeholder and on the home screen. The source of truth is
   audio2-www/src/lib/icons.ts (rules: audio2-www/docs/icon-system.md).

   In the app they are drawn without the brand's accent dot: in a list or a
   toolbar a coloured dot at the top right reads as an unread badge (icon
   system §6). Without it the vessel would sit low and left, so each is moved to
   the centre of the 24 grid. Same props as a lucide icon: size with className,
   colour with `currentColor`. */

export type Cloud = "book" | "podcast" | "music";

const VESSELS: Record<Cloud, { shift: string; body: React.ReactNode }> = {
  book: {
    shift: "translate(1.5 -1.7)",
    body: (
      <>
        <rect x="4.4" y="6.2" width="12.2" height="15" rx="1.8" />
        <rect x="7.4" y="10.1" width="6.2" height="1.9" rx=".95" fill="currentColor" stroke="none" />
        <path d="M4.4 18.2h12.2" />
      </>
    ),
  },
  podcast: {
    shift: "translate(0 -0.4)",
    body: (
      <>
        <path d="M9.5 6.5a2.75 2.75 0 0 1 5.5 0v5a2.75 2.75 0 0 1-5.5 0Z" />
        <path d="M6.25 11.25a5.75 5.75 0 0 0 11.5 0" />
        <path d="M12 17v4" />
      </>
    ),
  },
  music: {
    shift: "translate(2.5 -1.5)",
    body: (
      <>
        <path d="M8 18.5V8l7.5-1.75v10.25" />
        <circle cx="5.75" cy="18.5" r="2.25" />
        <circle cx="13.25" cy="16.5" r="2.25" />
      </>
    ),
  },
};

type IconProps = SVGProps<SVGSVGElement>;

function Glyph({ cloud, svgRef, className, ...rest }: IconProps & { cloud: Cloud; svgRef: React.Ref<SVGSVGElement> }) {
  return (
    <svg
      ref={svgRef}
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      width="24"
      height="24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
      {...rest}
    >
      <g transform={VESSELS[cloud].shift}>{VESSELS[cloud].body}</g>
    </svg>
  );
}

export const BookIcon = forwardRef<SVGSVGElement, IconProps>(function BookIcon(props, ref) {
  return <Glyph cloud="book" svgRef={ref} {...props} />;
});

export const PodcastIcon = forwardRef<SVGSVGElement, IconProps>(function PodcastIcon(props, ref) {
  return <Glyph cloud="podcast" svgRef={ref} {...props} />;
});

export const MusicIcon = forwardRef<SVGSVGElement, IconProps>(function MusicIcon(props, ref) {
  return <Glyph cloud="music" svgRef={ref} {...props} />;
});
