// SPDX-License-Identifier: AGPL-3.0-or-later
import { Link } from "react-router-dom";
import Logo from "../components/Logo";
import { useT } from "../i18n";

export default function NotFound() {
  const { t } = useT();
  return (
    <div className="flex min-h-dvh flex-col items-center justify-center bg-bg px-6 text-center text-fg">
      <Logo size={44} />
      <h1 className="mt-6 text-2xl font-semibold tracking-tight">{t("notFound.title")}</h1>
      <p className="mt-2 text-sm text-muted">{t("notFound.body")}</p>
      <Link to="/" className="mt-6 text-sm font-medium text-accent hover:underline">{t("notFound.back")}</Link>
    </div>
  );
}
