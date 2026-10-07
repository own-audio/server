# Memory use of the server — investigation, 2026-10-07

Why the server looked heavy, what it really needs, and how it compares with
Navidrome and Audiobookshelf. The target this serves is in
[CAPACITY.md](CAPACITY.md): a smaller footprint than both.

## Summary

- The server process needs **about 20 MiB** after start.
- The 200–270 MiB seen after a burst of work is memory that **glibc's
  allocator keeps** after the work is done, not data the server still holds.
- Three environment variables bring the peak under the full API test suite
  from **272 MiB to 28 MiB**, with every test passing at the same speed. They
  are not applied yet; see "Proposed change".
- With them, the server process is well below Navidrome and Audiobookshelf.
  The whole stack is not yet, because PostgreSQL comes on top.

## Method

- Image built from `main` (`backend/Dockerfile`), run in the compose stack on
  an Apple-silicon Mac (OrbStack, arm64), PostgreSQL 16 and RustFS beside it.
- Memory is the process's resident set (`VmRSS` of PID 1 in the container),
  not `docker stats`, which also counts page cache.
- Load: the conformance suite (`conformance/run.py`, 321 checks, about
  a minute). It signs in, uploads and deletes audiobooks and tracks, runs
  file sync, the trash, families and statistics. It is a burst, not a steady
  household, so it is a fair worst case for a family server.
- Each run starts from a freshly created container; "idle" is 45 s after
  start, "peak" is the highest one-second sample during the suite.

## Results

| glibc settings | Idle | Peak during the suite | 60 s after |
|---|---|---|---|
| default | 21 MiB | 272 MiB | 266 MiB |
| `MALLOC_ARENA_MAX=2` | 11 MiB | about 157 MiB | 150 MiB |
| `MALLOC_ARENA_MAX=2`, `MALLOC_MMAP_THRESHOLD_=131072`, `MALLOC_TRIM_THRESHOLD_=131072` | 17 MiB | **28 MiB** | 24 MiB |

With the last settings the suite passed (321 ok) in 56 s, against 57 s with
the defaults.

Elsewhere, for scale:

| Where | Server process |
|---|---|
| Hosted production API, normal day | 70 MiB |
| Hosted canary | 190 MiB |
| demo.own.audio during its nightly reset (seed, a year of history) | 159 MiB peak, limit 512 MiB |

## Why the defaults look heavy

The server is a Tokio program with one worker thread per CPU core. glibc
gives threads their own allocation arenas (up to 8 per core), and freed memory
stays in the arena it came from. Uploads, downloads, cover images and JSON
bodies are large, short-lived buffers; glibc's dynamic mmap threshold grows
after the first large free, so later large buffers come from the arenas too
and are never handed back. The process keeps the high-water mark of every
burst it has seen.

- `MALLOC_ARENA_MAX=2` caps the arenas, so freed memory is reused instead of
  spread across many pools.
- `MALLOC_MMAP_THRESHOLD_=131072` pins the threshold: anything over 128 KiB is
  its own mapping and goes back to the system when freed.
- `MALLOC_TRIM_THRESHOLD_=131072` returns free memory at the top of the heap
  sooner.

Nothing here is a leak: memory is flat across repeated runs, and idle memory
after start is the same in every configuration.

## Compared with Navidrome and Audiobookshelf

Same Mac, same day, each server indexing the demo's own files read-only
(28 songs for Navidrome, 2 books in 24 files for Audiobookshelf), then 20
rounds of browsing and search plus a few streams. Memory is the sum of
`VmRSS` over every process in the container.

| Server | Idle after scan | Peak under load | 30 s after |
|---|---|---|---|
| Navidrome 0.64.2 (Go, SQLite) | 86 MiB | 88 MiB | 60 MiB |
| Audiobookshelf 2.37.1 (Node.js, SQLite) | 78 MiB | 88 MiB | 97 MiB |
| own.audio server, default glibc | 21 MiB | 272 MiB | 266 MiB |
| own.audio server, tuned glibc | 17 MiB | 28 MiB | 24 MiB |

Caveats, so nobody quotes this wrongly:

- The loads differ. Our suite uploads and deletes media and is much heavier
  than browsing; the comparison is fair in direction, not to the megabyte.
- Both of them embed SQLite in their own process. **We also need
  PostgreSQL**: 70–130 MiB in the compose stack with stock settings
  (`shared_buffers` 128 MB, 100 connections). Process for process we are
  smaller; stack for stack we are not, yet.
- Tiny libraries. Navidrome and Audiobookshelf both grow with the catalog
  (Audiobookshelf keeps its library in memory); so do we today, through the
  unpaginated paths listed in [CAPACITY.md](CAPACITY.md).

## Is it fair to compare a PostgreSQL stack with SQLite servers?

Partly. It depends on the question being asked.

- **"How efficient is the server's own code?"** Comparing processes is fair
  in direction, as the table above does. A SQLite server's figure already
  contains its database engine and SQLite's page cache; ours does not, so it
  flatters us.
- **"How much RAM does the machine need?"** Only the whole stack is fair:
  every container the install cannot run without. For us that is the server
  and PostgreSQL. RustFS is not part of it: it is one optional store, and
  plain local storage or an existing S3 store replace it.
- **Like for like.** Our one server replaces two: a household that wants
  music and audiobooks runs Navidrome *and* Audiobookshelf. The fair
  comparison is our server plus PostgreSQL against both of theirs together:
  about 150–180 MiB for the two of them in the measurements above, against
  28 MiB plus PostgreSQL for us.

Measurement traps that make either side look better than it is:

- Summing `VmRSS` over PostgreSQL's processes counts its shared buffers once
  per connection. Use the container's cgroup memory (what `docker stats`
  shows, which already leaves out reclaimable file cache) or PSS instead.
- Both SQLite and PostgreSQL read data through the kernel's page cache. That
  memory is real but reclaimable and appears in no process's RSS, on either
  side; it is not a reason to prefer one database.
- PostgreSQL's memory is mostly configuration. Stock settings reserve 128 MB
  of shared buffers for any database size; a family install can run with 16–
  32 MB, and idle connections cost a few MB each, so a pool of 5–10 instead
  of 20 matters.
- Same catalog, same requests, same warm-up for every server; idle and peak
  both reported.

So the target in [CAPACITY.md](CAPACITY.md) is meant as **whole stack against
Navidrome and Audiobookshelf together**, measured as container memory with
the same catalog and load. That run has not been done yet; it needs the tuned
PostgreSQL settings first.

## What the tuned settings cost

Nothing functional: the server does exactly the same work, and nothing is
limited — not uploads, file sizes, streams or the number of users. The
settings only change *when* glibc hands memory back to the system.

| Setting | What it trades | Matters for a family? |
|---|---|---|
| Two arenas | Threads that allocate at the same moment can wait on each other's arena lock. | No: a family's few concurrent requests do not contend. The suite ran 1 s faster, not slower. |
| 128 KiB mmap threshold | Each buffer over 128 KiB (uploads, cover images, large JSON) is its own mapping: one `mmap` and one `munmap` system call and fresh zeroed pages each time, instead of reusing heap memory. | No: microseconds per large buffer. Audio is not proxied, so this is not on the streaming path. |
| 128 KiB trim threshold | Free memory at the top of the heap is returned sooner, so the next burst asks the system again. | No: same order of cost as above. |

Where it could start to matter is a busy multi-user server with many CPU
cores allocating in parallel — the hosted edition, not a family install.
There the setting is measured before it is adopted; the cost would show up as
CPU time and latency, never as wrong results.

Other notes:

- They are plain environment variables: an operator can override or remove
  them in `docker-compose.yml` without a new image.
- They apply to glibc only. The image is Debian (glibc); on a musl build
  (Alpine) they would be ignored and musl's own allocator applies.
- Lower resident memory also means a tight container limit (the demo runs
  with 512 MiB) has real headroom instead of filling up with retained
  memory.

## Proposed change (not applied)

1. Set the three variables in the runtime stage of `backend/Dockerfile`
   (and the hosted edition's Dockerfile), so every install gets them:

   ```dockerfile
   ENV MALLOC_ARENA_MAX=2 \
       MALLOC_MMAP_THRESHOLD_=131072 \
       MALLOC_TRIM_THRESHOLD_=131072
   ```

   The heavier alternative is a different global allocator (jemalloc or
   mimalloc as `#[global_allocator]`); not needed while the variables work.
2. Tune PostgreSQL for one family in the compose file: `shared_buffers=64MB`,
   `max_connections=30`, and a server pool of 10 instead of 20
   (`db/mod.rs`). Measure the stack again afterwards.
3. Remove the catalog-sized allocations listed in
   [CAPACITY.md](CAPACITY.md) before claiming flat memory for large
   catalogs.

## Reproducing

The scripts used live outside the repository; the method is short enough to
repeat by hand:

```bash
docker compose up -d
docker exec own-audio-foss-server-1 grep VmRSS /proc/1/status      # idle
python3 conformance/run.py --base-url http://127.0.0.1:8080 \
  --admin-email … --admin-password … --compose-dir .               # load
docker exec own-audio-foss-server-1 grep VmRSS /proc/1/status      # after
```

For the tuned run, add the three variables to the `server` service's
`environment` and recreate it.
