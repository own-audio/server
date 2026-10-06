// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";
import { Navigate } from "react-router-dom";
import { useServerFeatures } from "../lib/features";
import type { ServerFeatures } from "../api/server";

type BooleanFeature = { [K in keyof ServerFeatures]: ServerFeatures[K] extends boolean ? K : never }[keyof ServerFeatures];

/** Renders its children only on a server that offers `feature`; otherwise goes home. */
export default function FeatureGate({ feature, children }: { feature: BooleanFeature; children: ReactNode }) {
  const { features, isLoading } = useServerFeatures();
  if (isLoading) return null;
  if (!features[feature]) return <Navigate to="/" replace />;
  return <>{children}</>;
}
