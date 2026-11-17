// SPDX-License-Identifier: AGPL-3.0-or-later

/** The server's default too: the last word of the name, a surname. Invites read "the {name} family". */
export function defaultFamilyName(displayName: string): string {
  const words = displayName.trim().split(/\s+/);
  return words[words.length - 1] ?? "";
}
