// SPDX-License-Identifier: AGPL-3.0-or-later
import { User } from "lucide-react";
import AuthImage from "../AuthImage";
import { cn } from "../../lib/cn";

/** A round author photo — fetched from Wikimedia Commons server-side and
 *  cached, or one someone picked by hand. Falls back to a plain person icon
 *  while it loads, on a miss, and for an author nobody has a picture of. */
export function AuthorAvatar({ src, alt, className }: { src?: string | null; alt: string; className?: string }) {
  const placeholder = (
    <div className="flex h-full w-full items-center justify-center bg-bg-alt text-muted" aria-hidden="true">
      <User className="h-[45%] w-[45%] opacity-60" />
    </div>
  );
  return (
    <div className={cn("relative shrink-0 overflow-hidden rounded-full bg-bg-alt", className)}>
      {src ? <AuthImage src={src} alt={alt} className="h-full w-full object-cover" fallback={placeholder} /> : placeholder}
    </div>
  );
}
