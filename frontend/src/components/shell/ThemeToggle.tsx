// SPDX-License-Identifier: AGPL-3.0-or-later
import { Monitor, Moon, Sun } from "lucide-react";
import { useSyncExternalStore } from "react";
import { useTheme, type ThemePreference } from "../../lib/theme";
import { SegmentedControl } from "../ui/SegmentedControl";
import { Tooltip } from "../ui/Tooltip";
import { IconButton } from "../ui/Button";
import { useT, type PlainKey } from "../../i18n";

const ORDER: ThemePreference[] = ["system", "light", "dark"];
const LABEL: Record<ThemePreference, PlainKey> = { system: "common.theme.system", light: "common.theme.light", dark: "common.theme.dark" };
const ICON: Record<ThemePreference, React.ReactNode> = {
  system: <Monitor className="h-4 w-4" />,
  light: <Sun className="h-4 w-4" />,
  dark: <Moon className="h-4 w-4" />,
};

const DARK_QUERY = "(prefers-color-scheme: dark)";

function subscribeToSystemTheme(onChange: () => void) {
  const mq = window.matchMedia(DARK_QUERY);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

/**
 * The signed-out screens offer only Light and Dark: "Auto" means nothing to
 * someone who hasn't signed in yet. They still start on the system's look, and
 * choosing the system's look again goes back to following it rather than
 * pinning it.
 */
function BinaryThemeToggle({ compact }: { compact?: boolean }) {
  const { preference, setPreference } = useTheme();
  const system = useSyncExternalStore(subscribeToSystemTheme, () =>
    window.matchMedia(DARK_QUERY).matches ? "dark" : "light",
  );
  const current = preference === "system" ? system : preference;
  const choose = (theme: "light" | "dark") => setPreference(theme === system ? "system" : theme);
  const { t } = useT();

  if (compact) {
    const next = current === "dark" ? "light" : "dark";
    return (
      <IconButton label={t("shell.theme.switchTo", { mode: t(LABEL[next]).toLowerCase() })} onClick={() => choose(next)} className="h-10 w-10">
        {current === "dark" ? <Moon className="h-4 w-4" /> : <Sun className="h-4 w-4" />}
      </IconButton>
    );
  }

  return (
    <SegmentedControl<"light" | "dark">
      size="sm"
      value={current}
      onChange={choose}
      segments={[
        { value: "light", label: <><Sun /> {t("common.theme.light")}</>, title: t("common.theme.light") },
        { value: "dark", label: <><Moon /> {t("common.theme.dark")}</>, title: t("common.theme.dark") },
      ]}
    />
  );
}

/**
 * `binary` drops "Auto" for the signed-out screens, see BinaryThemeToggle.
 *
 * `compact` is for the collapsed sidebar rail, where three segments don't fit
 * in 68px — a segmented control there clips to whichever option happens to be
 * first, leaving the other two unreachable. One button that cycles keeps all
 * three within reach.
 */
export default function ThemeToggle({ compact, binary }: { compact?: boolean; binary?: boolean }) {
  const { preference, setPreference } = useTheme();
  const { t } = useT();

  if (binary) return <BinaryThemeToggle compact={compact} />;

  if (compact) {
    const next = ORDER[(ORDER.indexOf(preference) + 1) % ORDER.length];
    return (
      <Tooltip label={t("shell.theme.current", { current: t(LABEL[preference]), next: t(LABEL[next]) })} side="right">
        <IconButton
          label={t("shell.theme.current", { current: t(LABEL[preference]), next: t(LABEL[next]) })}
          onClick={() => setPreference(next)}
          className="mx-auto h-10 w-10"
        >
          {ICON[preference]}
        </IconButton>
      </Tooltip>
    );
  }

  return (
    <SegmentedControl<ThemePreference>
      size="sm"
      value={preference}
      onChange={setPreference}
      segments={[
        { value: "system", label: <><Monitor /> {t("common.theme.system")}</>, title: t("common.theme.system") },
        { value: "light", label: <><Sun /> {t("common.theme.light")}</>, title: t("common.theme.light") },
        { value: "dark", label: <><Moon /> {t("common.theme.dark")}</>, title: t("common.theme.dark") },
      ]}
    />
  );
}
