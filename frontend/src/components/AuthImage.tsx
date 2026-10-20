// SPDX-License-Identifier: AGPL-3.0-or-later
import React, { useEffect, useState } from "react";
import { useAuthStore } from "../store/authStore";
import { loadAuthImage } from "../lib/authImages";

type Props = React.ImgHTMLAttributes<HTMLImageElement> & {
  src: string;
  /** Rendered when there is nothing to show — no src, still loading, or the
   *  fetch came back with something that isn't an image. */
  fallback?: React.ReactNode;
};

export default function AuthImage({ src, alt, className, fallback, ...rest }: Props) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const token = useAuthStore.getState().token;

  useEffect(() => {
    if (!src) return;
    let mounted = true;
    // Shared, queued and cached across every image on the page (lib/authImages.ts).
    const { url, release } = loadAuthImage(src, token);
    void url.then((u) => mounted && u && setObjectUrl(u));
    return () => {
      mounted = false;
      release();
      setObjectUrl(null);
    };
  }, [src, token]);

  if (!src) return <>{fallback ?? null}</>;
  if (!objectUrl) return <>{fallback ?? <div className={className} />}</>;
  return <img src={objectUrl} alt={alt} className={className} {...rest} />;
}
