let settable: boolean | undefined;

/** iOS Safari ignores `volume` on media elements (it reads back 1; only the
 *  hardware buttons change it), so a slider there moves and does nothing.
 *  `muted` does work everywhere. */
export function canSetVolume(): boolean {
  if (settable === undefined) {
    try {
      const probe = new Audio();
      probe.volume = 0.5;
      settable = probe.volume === 0.5;
    } catch {
      settable = true;
    }
  }
  return settable;
}

export function applyVolume(audio: HTMLMediaElement, volume: number, muted: boolean) {
  audio.muted = muted;
  audio.volume = volume;
}
