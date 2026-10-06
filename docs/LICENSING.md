# Why AGPL, and what was considered

Decided 2026-10-06. Not legal advice; a lawyer should read this before the
repository goes public.

## The goals the licence has to serve

1. Self-hosters can run, change and share the server freely, and the
   project is accepted where self-hosters look: GitHub, Docker Hub,
   awesome-selfhosted, Linux distributions. That means an **OSI-approved**
   licence; "source-available" licences are listed separately or excluded.
2. Nobody can take the server, improve it privately and run it as a
   competing hosted service without giving the improvements back.
3. own.audio keeps running a hosted edition that combines this code with a
   private billing layer, and keeps the option to license the code to a
   company that cannot accept copyleft.
4. The brand stays with the project.

## The choice: AGPL-3.0-or-later + CLA + trademark notice

| Licence | Goal 1 | Goal 2 | Goal 3 | Notes |
|---|---|---|---|---|
| **AGPL-3.0** | yes | **yes** — §13 extends copyleft to network use | yes, with a CLA | Immich, Nextcloud, Mastodon, Grafana, Plausible. The standard answer for a self-hosted server with a hosted twin. |
| GPL-3.0 | yes | **no** — the "ASP loophole": a hosted competitor never distributes, so never has to share | yes, with a CLA | Audiobookshelf, Navidrome, Jellyfin. Fine for them: none has a hosted business to protect. |
| MIT / Apache-2.0 | yes | **no** — anyone may host a closed fork | yes | Maximises adoption and contributions; gives away goal 2 entirely. |
| BSL 1.1, Elastic 2.0, SSPL | **no** — not open source; awesome-selfhosted and most distros exclude them; "fake open source" reputation | yes | yes | Reasonable for a venture-backed database company; wrong signal for a one-person family audio server. |
| FSL (Functional Source License) | **no** (fair source, not OSI) | yes for two years, then Apache | yes | Elegant, but the same acceptance problem as BSL today. |

AGPL is the only OSI licence that meets goal 2. It does **not** meet a goal
Kornel did not actually have but might assume: it does not stop anyone from
running the unmodified server, commercially included, or even hosting it for
others. What it stops is keeping *changes* private while offering them over a
network. The trademark notice is what stops them doing so under the own.audio
name.

**"or-later" rather than "only":** the FSF recommends it, `mindmapvault-server`
uses it, and it keeps the code combinable with a future AGPL v4 without
re-licensing every contributor's work. The cost is trusting the FSF's future
drafting; most of the ecosystem accepts that trade.

## Why a CLA

Kornel's own code may be combined with the private hosted layer under any
terms he likes — the copyright holder is not bound by his own licence. A
third party's contribution is different: it arrives under the AGPL, and
combining it into the hosted binary would make the whole hosted service a
derivative that must be offered to its users under the AGPL — billing layer
included. A short contributor licence agreement ("you grant Kornel Maráz a
perpetual right to use your contribution under any licence, and you keep
your copyright") removes that, and also keeps goal 3 (a commercial licence
for a company that cannot ship AGPL code) possible later.

The cost: CLAs deter some contributors. The alternative is a DCO
(Developer Certificate of Origin) with no relicensing grant, which is
friendlier but gives up the hosted combination and the dual-licence option.
For this project the hosted edition exists today, so the CLA wins. Use a
standard text (the Apache ICLA or the Harmony individual agreement) and a
bot that checks a signature comment on each pull request.

## Clients are a separate question

This document covers the server. The native clients are not in this
repository and are not AGPL. If they are ever open-sourced, **not under the
GPL family**: the FSF's position is that Apple's App Store terms are
incompatible with the GPL, and apps have been pulled over it. MIT or
Apache-2.0 for a client, if at all. Keeping them proprietary is also fine;
the API contract is public either way.

## Third-party code

The server depends on crates under MIT, Apache-2.0, BSD and MPL-2.0 licences,
all compatible with AGPL distribution. `cargo deny` (Phase 6) enforces an
allow-list so a GPL-incompatible dependency cannot slip in. `yt-dlp` and
`ffmpeg` are separate programs invoked at runtime (Unlicense and
LGPL/GPL respectively), not linked, and are installed in the image from
Debian packages.
