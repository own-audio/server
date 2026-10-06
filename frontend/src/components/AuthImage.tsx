// SPDX-License-Identifier: AGPL-3.0-or-later
import React, { useEffect, useState } from "react";
import { useAuthStore } from "../store/authStore";
import { mediaUrl } from "../api/client";
import { offlineCoverUrl } from "../lib/offline/downloads";

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
    let createdUrl: string | null = null;

    const show = (url: string) => {
      if (!mounted) {
        URL.revokeObjectURL(url);
        return;
      }
      createdUrl = url;
      setObjectUrl(url);
    };

    // A cover kept on the device for offline listening is used as is: no
    // request at all, which is what lets an offline player open without one.
    void offlineCoverUrl(src).then((stored) => {
      if (stored) {
        show(stored);
        return;
      }
      fetch(mediaUrl(src) as string, {
        headers: token ? { Authorization: `Bearer ${token}` } : undefined,
      })
        .then((res) => {
          if (!res.ok) throw new Error(`Image fetch failed: ${res.status}`);
          // A 200 is not proof of an image: an SPA fallback answers every path
          // with index.html, and blob-ifying that yields an <img> that silently
          // fails to decode instead of falling back to the placeholder.
          const type = res.headers.get("content-type") ?? "";
          if (!type.startsWith("image/")) throw new Error(`Not an image: ${type}`);
          return res.blob();
        })
        .then((blob) => show(URL.createObjectURL(blob)))
        .catch(() => {
          // ignore — caller may render fallback
        });
    });

    return () => {
      mounted = false;
      if (createdUrl) URL.revokeObjectURL(createdUrl);
      setObjectUrl(null);
    };
  }, [src, token]);

  if (!src) return <>{fallback ?? null}</>;
  if (!objectUrl) return <>{fallback ?? <div className={className} />}</>;
  return <img src={objectUrl} alt={alt} className={className} {...rest} />;
}
