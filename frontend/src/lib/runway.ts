// SPDX-License-Identifier: AGPL-3.0-or-later

/** Past this, a count of days (and a run-out date thousands of years away, as a tiny library
 *  gives) says nothing a person can use. */
export const LONG_RUNWAY_DAYS = 3650;

export function isLongRunway(daysRemaining: number | null | undefined): boolean {
  return daysRemaining != null && daysRemaining > LONG_RUNWAY_DAYS;
}
