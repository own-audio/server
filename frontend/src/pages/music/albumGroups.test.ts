// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { albumsOf, artistAlbumKey } from "./albumGroups";
import type { MusicTrack } from "../../api/types";

const track = (id: string, album: string | null, album_id: string | null, cover: string | null = null) =>
  ({ id, title: id, album, album_id, cover_url: cover }) as unknown as MusicTrack;

describe("albumsOf", () => {
  it("groups an artist's songs by album, by name, each with its count and first cover", () => {
    const albums = albumsOf(
      [
        track("1", "Twenty-One", "a21"),
        track("2", "Twenty-One", "a21", "/c21"),
        track("3", "Nineteen", "a19", "/c19"),
        track("4", "Twenty-One", "a21", "/other"),
      ],
      "Unknown Album",
    );
    expect(albums).toEqual([
      { key: "a19", title: "Nineteen", count: 1, cover: "/c19", own: true },
      { key: "a21", title: "Twenty-One", count: 3, cover: "/c21", own: true },
    ]);
  });

  it("keeps two albums of one name apart by their ids, and songs without an id by name", () => {
    const albums = albumsOf([track("1", "Live", "x"), track("2", "Live", "y"), track("3", "Demo", null), track("4", null, null)], "Unknown Album");
    expect(albums.map((a) => [a.key, a.title, a.count])).toEqual([
      ["name:Demo", "Demo", 1],
      ["x", "Live", 1],
      ["y", "Live", 1],
      ["name:", "Unknown Album", 1],
    ]);
    expect(artistAlbumKey(track("5", "Demo", null))).toBe("name:Demo");
  });

  it("puts the artist's own albums first and the compilations they appear on after", () => {
    const song = (id: string, album: string, album_artist: string) =>
      ({ id, title: id, album, album_id: album, artist: "Aerosmith", album_artist, cover_url: null }) as unknown as MusicTrack;
    const albums = albumsOf(
      [
        song("1", "70s Rock Essentials", "Various Artists"),
        song("2", "Rock for the Rising Sun", "Aerosmith"),
        song("3", "Joint Venture", "Run-DMC; aerosmith"),
      ],
      "Unknown Album",
      "Aerosmith",
    );
    expect(albums.map((a) => [a.title, a.own])).toEqual([
      ["Joint Venture", true],
      ["Rock for the Rising Sun", true],
      ["70s Rock Essentials", false],
    ]);
  });
});
