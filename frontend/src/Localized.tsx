// SPDX-License-Identifier: AGPL-3.0-or-later
import App from "./App";
import { Toaster } from "./components/ui/Toast";
import { useI18n } from "./i18n";

/* A language change re-renders the whole app from here, so text produced by
   helpers outside React follows too. Nothing is lost: data lives in the query
   cache and the stores, not in the tree. */
export default function Localized() {
  const locale = useI18n((s) => s.locale);
  return (
    <div key={locale} className="contents">
      <App />
      <Toaster />
    </div>
  );
}
