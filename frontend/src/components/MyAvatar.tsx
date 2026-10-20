// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";
import AuthImage from "./AuthImage";
import { useAuthStore } from "../store/authStore";
import { useAvatarVersion } from "../lib/avatar";

/** The signed-in user's picture, or `fallback` when there is none (the server answers 404). */
export default function MyAvatar({ className, fallback }: { className?: string; fallback: ReactNode }) {
  const id = useAuthStore((s) => s.user?.id);
  const version = useAvatarVersion((s) => s.version);
  if (!id) return <>{fallback}</>;
  return <AuthImage key={version} src={`/api/v1/users/${id}/avatar?v=${version}`} alt="" className={className} fallback={fallback} />;
}
