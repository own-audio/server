// SPDX-License-Identifier: AGPL-3.0-or-later
import axios from "axios";
import api from "./client";

export type UploadKind =
  | "audiobook_file"
  | "audiobook_cover"
  | "music_track"
  | "music_cover";

export interface PresignResponse {
  object_key: string;
  url: string;
  method: string;
  content_type: string;
  expires_in_secs: number;
}

export async function presignUpload(req: {
  kind: UploadKind;
  filename: string;
  contentType?: string;
  sizeBytes?: number;
}): Promise<PresignResponse> {
  const { data } = await api.post<PresignResponse>("/uploads/presign", {
    kind: req.kind,
    filename: req.filename,
    content_type: req.contentType?.trim() || "application/octet-stream",
    size_bytes: req.sizeBytes,
  });
  return data;
}

/**
 * PUT the bytes straight to object storage.
 *
 * Deliberately uses a bare axios call, not the shared `api` instance: that one
 * attaches an Authorization header to every request, which would collide with
 * the presigned URL's own signature. The Content-Type must match the one the
 * server signed exactly, or the PUT is rejected as a signature mismatch.
 */
export async function putToStorage(
  presigned: PresignResponse,
  file: File,
  onProgress?: (fraction: number) => void
): Promise<void> {
  await axios.put(presigned.url, file, {
    headers: { "Content-Type": presigned.content_type },
    onUploadProgress: (event) => {
      if (onProgress && event.total) onProgress(event.loaded / event.total);
    },
  });
}

export interface CompletedUpload {
  media_object_id: string;
  object_key: string;
  content_type: string;
  size_bytes: number;
}

/** Register an uploaded object as a media_object. */
export async function completeUpload(objectKey: string): Promise<CompletedUpload> {
  const { data } = await api.post<CompletedUpload>("/uploads/complete", {
    object_key: objectKey,
  });
  return data;
}

/** presign → PUT, returning the key to hand back to the API. */
export async function uploadToStorage(
  kind: UploadKind,
  file: File,
  relativePath?: string,
  onProgress?: (fraction: number) => void
): Promise<string> {
  const presigned = await presignUpload({
    kind,
    filename: relativePath || file.name,
    contentType: file.type,
    sizeBytes: file.size,
  });
  await putToStorage(presigned, file, onProgress);
  return presigned.object_key;
}
