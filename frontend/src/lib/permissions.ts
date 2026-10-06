// SPDX-License-Identifier: AGPL-3.0-or-later
import { useQuery } from "@tanstack/react-query";
import { getFamily, isFamilyAdmin } from "../api/family";

/**
 * What this account may do, from `GET /family`'s `my_can_upload` /
 * `my_can_generate`.
 *
 * Hiding a button is UX, never a control — the server refuses the request
 * either way (`docs/family-permissions-plan.md`). So while the answer is
 * unknown these read permissive: showing an affordance that turns out to be
 * refused is better than flickering one away from someone who has it, and the
 * refusal still arrives as a clear message.
 */
export function useMyPermissions() {
  const { data, isLoading } = useQuery({ queryKey: ["family"], queryFn: getFamily, retry: false });
  return {
    canUpload: data?.my_can_upload ?? true,
    canGenerate: data?.my_can_generate ?? true,
    /** Not permissive while loading, unlike the two above: it unlocks deleting
     *  other people's items, which is better appearing a moment late. */
    isFamilyAdmin: isFamilyAdmin(data?.my_role),
    isLoading,
  };
}
