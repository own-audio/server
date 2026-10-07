# Editions

The same server runs in two places. This repository is the whole of what you
run yourself. The hosted service at [own.audio](https://www.own.audio) runs
this code plus a private layer for the things that only make sense for a
paid service. Clients cannot tell the two apart; they read
`GET /api/v1/server` and show what it offers.

| | This server | own.audio hosted |
|---|---|---|
| Audiobooks, podcasts, music, playlists, statistics | yes | yes |
| Web console, OpenSubsonic API, the native apps | yes | yes |
| Families | one per install, any number of members | one per account |
| Read-only library folders, local storage | yes | — |
| S3-compatible storage | optional | yes |
| Sign-in with Google, Apple, Microsoft | optional, your own client ids | as the operator enables them |
| Invite mail | optional, your SMTP server | the operator's mail server |
| Music identify | yes, through the public MusicBrainz API (one request a second) | yes (a private metadata service) |
| Podcast discovery: search, categories, similar shows | — | yes (the same service) |
| Narrate a book, translate a podcast episode | — | yes, paid per use |
| Storage billing, credit, payments | — | yes |

What is not here, and why, so nobody has to ask:

- **Payments, credit and storage billing.** There is nothing to bill on your
  own server.
- **Narration and translation.** They call paid text-to-speech and
  translation services on the operator's account, metered per family; they
  live with the billing they depend on.
- **The private metadata service.** A MusicBrainz mirror and its own index,
  too big to bundle. This edition identifies music through the public
  MusicBrainz API instead, which is slower (MusicBrainz allows one request a
  second) and has no podcast catalogue.

The split is made at one place in the code, `backend/src/hooks.rs`: the
hosted edition plugs into it and adds routes, jobs and feature flags. Nothing
in this repository checks which edition it is. The full reasoning is in
[SCOPE.md](SCOPE.md) and [LICENSING.md](LICENSING.md).
