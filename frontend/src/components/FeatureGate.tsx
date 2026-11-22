// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";
import { Navigate } from "react-router-dom";
import { useServerFeatures } from "../lib/features";
import type { ServerFeatures } from "../api/server";

type BooleanFeature = { [K in keyof ServerFeatures]-?: ServerFeatures[K] extends boolean ? K : never }[keyof ServerFeatures];

/** Renders its children only on a server that offers `feature` (or passes `when`); otherwise goes home. */
export default function FeatureGate({
  feature,
  when,
  children,
}: {
  feature?: BooleanFeature;
  when?: (features: ServerFeatures) => boolean;
  children: ReactNode;
}) {
  const { features, isLoading } = useServerFeatures();
  if (isLoading) return null;
  const offered = when ? when(features) : feature ? !!features[feature] : false;
  if (!offered) return <Navigate to="/" replace />;
  return <>{children}</>;
}
