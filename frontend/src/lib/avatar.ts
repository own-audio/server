// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";

// The picture's address never changes when a new one is uploaded, so a
// version in the query makes every place showing it fetch the new one.
export const useAvatarVersion = create<{ version: number }>(() => ({ version: Date.now() }));

export function avatarChanged() {
  useAvatarVersion.setState({ version: Date.now() });
}
