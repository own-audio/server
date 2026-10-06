// SPDX-License-Identifier: AGPL-3.0-or-later
import { useRef, useState } from "react";
import { BookUp, FileText } from "lucide-react";
import type { DragEvent } from "react";
import { isSupportedSourceFile } from "../../../api/generation";
import { intlLocale, useT } from "../../../i18n";

function formatBytes(bytes: number): string {
  const one = (n: number) => n.toLocaleString(intlLocale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 });
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${one(bytes / 1024)} KB`;
  return `${one(bytes / (1024 * 1024))} MB`;
}

export default function UploadStep({
  file,
  onFileSelected,
}: {
  file: File | null;
  onFileSelected: (file: File) => void;
}) {
  const { t } = useT();
  const [dragActive, setDragActive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  function handleFile(candidate: File | undefined) {
    if (!candidate) return;
    if (!isSupportedSourceFile(candidate)) {
      setError(t("generate.upload.unsupported"));
      return;
    }
    setError(null);
    onFileSelected(candidate);
  }

  function handleDrop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setDragActive(false);
    handleFile(event.dataTransfer.files[0]);
  }

  return (
    <div>
      <h2 className="mb-1 text-lg font-semibold">{t("generate.upload.title")}</h2>
      <p className="mb-5 text-sm text-muted">{t("generate.upload.formats")}</p>

      <div
        onDragOver={(event) => {
          event.preventDefault();
          setDragActive(true);
        }}
        onDragLeave={() => setDragActive(false)}
        onDrop={handleDrop}
        className={`rounded-2xl border-2 border-dashed px-5 py-10 text-center transition ${
 dragActive
 ? "border-accent bg-accent/6 "
 : "border-border bg-bg-alt "
 }`}
      >
        {file ? (
          <>
            <FileText className="mx-auto h-10 w-10 text-accent-text" aria-hidden />
            <p className="mt-3 font-medium">{file.name}</p>
            <p className="text-sm text-muted">{formatBytes(file.size)}</p>
            <button
              type="button"
              onClick={() => inputRef.current?.click()}
              className="mt-4 rounded-full border border-border px-4 py-2 text-sm font-medium hover:border-accent hover:text-accent"
            >
              {t("generate.upload.chooseDifferent")}
            </button>
          </>
        ) : (
          <>
            <BookUp className="mx-auto h-10 w-10 text-muted" aria-hidden />
            <p className="mt-3 text-base font-semibold">{t("generate.upload.drop")}</p>
            <p className="mt-1 text-sm text-muted">{t("generate.upload.or")}</p>
            <button
              type="button"
              onClick={() => inputRef.current?.click()}
              className="mt-3 rounded-full bg-accent px-4 py-2 text-sm font-medium text-on-accent hover:brightness-110"
            >
              {t("generate.upload.chooseFile")}
            </button>
          </>
        )}
        <input
          ref={inputRef}
          type="file"
          accept=".epub,.mobi,.txt"
          className="hidden"
          onChange={(event) => handleFile(event.target.files?.[0])}
        />
      </div>

      {error && <p className="mt-2 text-sm text-error">{error}</p>}
    </div>
  );
}
