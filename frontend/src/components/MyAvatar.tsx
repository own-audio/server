// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import AuthImage from "./AuthImage";
import { getMyProfile } from "../api/auth";
import { useAuthStore } from "../store/authStore";
import { useAvatarVersion } from "../lib/avatar";

/** The signed-in user's picture, or `fallback` when there is none. */
export default function MyAvatar({ className, fallback }: { className?: string; fallback: ReactNode }) {
  const id = useAuthStore((s) => s.user?.id);
  const version = useAvatarVersion((s) => s.version);
  // Asked first, so an account without a picture is not a 404 on every page.
  const me = useQuery({ queryKey: ["my-profile", id, version], queryFn: getMyProfile, enabled: !!id, staleTime: 5 * 60_000 });
  const url = me.data?.avatar_url;
  if (!url) return <>{fallback}</>;
  return <AuthImage key={version} src={`${url}?v=${version}`} alt="" className={className} fallback={fallback} />;
}
