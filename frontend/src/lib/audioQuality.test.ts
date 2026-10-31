// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { describeDelivery, describeSource, playbackInfoFor, rememberPlaybackInfo } from "./audioQuality";

describe("describeSource", () => {
  it("gives a lossless file its bit depth and sample rate", () => {
    expect(describeSource({ content_type: "audio/flac", codec: "flac", sample_rate_hz: 96000, bit_depth: 24, channels: 2, bitrate_kbps: 2219 }))
      .toBe("FLAC · 24-bit / 96 kHz");
  });

  it("gives a lossy file its bitrate and sample rate, and says mono", () => {
    expect(describeSource({ content_type: "audio/mpeg", codec: "mp2", sample_rate_hz: 44100, channels: 1, bitrate_kbps: 128 }))
      .toBe("MP2 · 128 kbps · 44.1 kHz · mono");
  });

  it("does not call an unread audio/mpeg file MP3: it may be MP2", () => {
    expect(describeSource({ content_type: "audio/mpeg", bitrate_kbps: 129 })).toBe("129 kbps");
    expect(describeSource({ content_type: "audio/flac" })).toBe("FLAC");
  });

  it("names more than two channels, and an unknown codec as it is", () => {
    expect(describeSource({ content_type: "audio/x-foo", codec: "dts", channels: 6 })).toBe("DTS · 6 channels");
  });

  it("says nothing when nothing is known", () => {
    expect(describeSource(undefined)).toBeNull();
    expect(describeSource({ content_type: "application/octet-stream" })).toBeNull();
  });
});

describe("describeDelivery", () => {
  it("speaks only of a conversion", () => {
    expect(describeDelivery({ url: "u", decision: "direct" })).toBeNull();
    expect(describeDelivery({ url: "u", decision: "transcode", content_type: "audio/mpeg", bitrate_kbps: 192 }))
      .toBe("playing as MP3 · 192 kbps, converted by the server");
  });
});

describe("playbackInfoFor", () => {
  it("finds the answer for a link, also on the proxy route", () => {
    rememberPlaybackInfo({ url: "https://s/api/v2/stream?i=1&f=orig", decision: "direct" });
    expect(playbackInfoFor("https://s/api/v2/stream?i=1&f=orig&route=proxy")?.decision).toBe("direct");
    expect(playbackInfoFor("blob:xyz")).toBeUndefined();
  });
});
