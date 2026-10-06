// SPDX-License-Identifier: AGPL-3.0-or-later
import { useQuery } from "@tanstack/react-query";
import { getServerInfo, NO_FEATURES, type ServerFeatures, type ServerInfo } from "../api/server";

/**
 * The server's `features` (`GET /server`), cached for the session. Gating
 * on these is UX, not a control: the server answers 501 for a feature it
 * does not offer whatever the client shows. While the answer is unknown
 * everything optional reads false, so a self-hosted console never flashes a
 * Billing page it is about to hide.
 */
export function useServerFeatures(): { features: ServerFeatures; info?: ServerInfo; isLoading: boolean } {
  const { data, isLoading } = useQuery({
    queryKey: ["server"],
    queryFn: getServerInfo,
    staleTime: 60 * 60 * 1000,
    retry: false,
  });
  return { features: data?.features ?? NO_FEATURES, info: data, isLoading };
}
