// The icon of each kind of library folder, for the library page and the dialog that adds folders.
import AudioWaveform from "@lucide/svelte/icons/audio-waveform";
import Music from "@lucide/svelte/icons/eighth-notes";
import Film from "@lucide/svelte/icons/film";
import Folder from "@lucide/svelte/icons/folder";
import Image from "@lucide/svelte/icons/image";
import Palette from "@lucide/svelte/icons/palette";
import type { LibraryKind } from "#shared/protocol.ts";

export const KIND_ICON: Record<LibraryKind, typeof Music> = { music: Music, sfx: AudioWaveform, graphics: Image, luts: Palette, footage: Film, other: Folder };
