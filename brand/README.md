# Brand assets

The own.audio mark — a violet core, two sound arcs, and five family members
(a head with two shoulders each) around it — and the own.audio name are
trademarks of Kornel Maráz. **They are not licensed under the AGPL.** See
[TRADEMARK.md](../TRADEMARK.md).

They are in this repository so that unmodified builds of this project can
show where they come from. You may use them:

- in an unmodified build of this server, as the console and image ship them;
- to say that something is built for, compatible with, or based on own.audio.

If you fork the server into a product of your own, replace these files with
your own mark before you publish.

The single source of truth for the geometry is `src/lib/mark.ts` in the
marketing-site repository (`markDetailed`); the SVGs here reproduce it
exactly and are regenerated from it, never edited by hand. The colours are
the tuned family set, never raw RGB primaries.

| File | Use |
|---|---|
| `own-audio-mark.svg` | the mark; the arcs take the CSS `color` of their context (`currentColor`), so one file serves both themes |
| `own-audio-mark-on-light.svg` | the mark with dark-grey arcs, for light backgrounds where CSS colour is not available |
| `own-audio-mark-on-dark.svg` | the mark with light-grey arcs, for dark backgrounds |
| `own-audio-tile-180.png` | app-icon form: white mark on the violet tile (solid, as touch icons require) |
| `own-audio-lockup-640x160.png` | horizontal lockup, mark + "own.audio" wordmark, for headers and e-mail |
| `github-avatar-512.png` | the tile form at 512 px, the GitHub organisation's avatar |
| `github-social-preview.png` | 1280 × 640 card shown when the repository link is shared |

The favicon in the console is a separate, enlarged geometry for 16 and 32
pixels (`markFavicon` / `markFaviconArcs` in the same file); it is not the
mark and is generated with the console's own icon script.
