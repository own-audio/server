// SPDX-License-Identifier: AGPL-3.0-or-later
import { Navigate, useLocation } from "react-router-dom";
import { loginPathFor } from "../lib/returnTo";
import { useAuthStore } from "../store/authStore";
import { useQuery } from "@tanstack/react-query";
import { getSetupStatus } from "../api/setup";

export default function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const token = useAuthStore((s) => s.token);
  const location = useLocation();

  // Check if initial setup is complete (at least one user exists)
  const { data: setupStatus, isLoading } = useQuery({
    queryKey: ["setup-status"],
    queryFn: getSetupStatus,
    staleTime: 60_000,
    retry: 1,
  });

  // While loading, show nothing (avoids flash)
  if (isLoading) return null;

  // If setup not complete, redirect to the setup wizard
  if (setupStatus && !setupStatus.setup_complete) {
    return <Navigate to="/setup" replace />;
  }

  // If not logged in, redirect to login
  if (!token) return <Navigate to={loginPathFor(location)} replace />;

  return <>{children}</>;
}
