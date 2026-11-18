import BookOpen from "@lucide/svelte/icons/book-open";
import Briefcase from "@lucide/svelte/icons/briefcase";
import Building2 from "@lucide/svelte/icons/building-complex";
import Camera from "@lucide/svelte/icons/camera";
import Clapperboard from "@lucide/svelte/icons/clapperboard";
import Coffee from "@lucide/svelte/icons/coffee";
import Dumbbell from "@lucide/svelte/icons/dumbbell";
import Film from "@lucide/svelte/icons/film";
import Folder from "@lucide/svelte/icons/folder";
import Gamepad2 from "@lucide/svelte/icons/gamepad-2";
import Globe from "@lucide/svelte/icons/globe";
import GraduationCap from "@lucide/svelte/icons/graduation-cap";
import Headphones from "@lucide/svelte/icons/headphones";
import Heart from "@lucide/svelte/icons/heart";
import Megaphone from "@lucide/svelte/icons/megaphone";
import Mic from "@lucide/svelte/icons/mic";
import Music from "@lucide/svelte/icons/eighth-notes";
import Newspaper from "@lucide/svelte/icons/newspaper";
import Palette from "@lucide/svelte/icons/palette";
import Plane from "@lucide/svelte/icons/plane";
import Podcast from "@lucide/svelte/icons/mic-signal";
import Radio from "@lucide/svelte/icons/radio";
import Rocket from "@lucide/svelte/icons/rocket";
import Sparkles from "@lucide/svelte/icons/sparkles";
import Star from "@lucide/svelte/icons/star";
import Store from "@lucide/svelte/icons/store";
import Tv from "@lucide/svelte/icons/tv";
import Users from "@lucide/svelte/icons/users";
import Utensils from "@lucide/svelte/icons/utensils";
import Video from "@lucide/svelte/icons/video";
import type { Component } from "svelte";
import type { CollectionIcon, IconColor } from "#shared/protocol.ts";

/** The lucide icons a workspace or series can have, by their lucide name. */
export const LUCIDE_ICONS: Record<string, Component<{ class?: string }>> = {
  folder: Folder,
  briefcase: Briefcase,
  "building-2": Building2,
  store: Store,
  users: Users,
  megaphone: Megaphone,
  mic: Mic,
  podcast: Podcast,
  radio: Radio,
  headphones: Headphones,
  music: Music,
  clapperboard: Clapperboard,
  film: Film,
  video: Video,
  camera: Camera,
  tv: Tv,
  "gamepad-2": Gamepad2,
  newspaper: Newspaper,
  "book-open": BookOpen,
  "graduation-cap": GraduationCap,
  palette: Palette,
  sparkles: Sparkles,
  star: Star,
  heart: Heart,
  rocket: Rocket,
  globe: Globe,
  plane: Plane,
  coffee: Coffee,
  utensils: Utensils,
  dumbbell: Dumbbell,
};

export const EMOJI_SUGGESTIONS = ["🎙️", "🎬", "📺", "🎵", "🚀", "💍", "🍳", "🎮", "📚", "🏋️", "✈️", "⭐"];

/** Tile and glyph colors per icon color, readable in light and dark. */
export const ICON_COLOR_CLASS: Record<IconColor, string> = {
  gray: "bg-zinc-500/15 text-zinc-600 dark:text-zinc-300",
  red: "bg-red-500/15 text-red-600 dark:text-red-400",
  orange: "bg-orange-500/15 text-orange-600 dark:text-orange-400",
  amber: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
  green: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400",
  teal: "bg-teal-500/15 text-teal-700 dark:text-teal-400",
  blue: "bg-blue-500/15 text-blue-600 dark:text-blue-400",
  violet: "bg-violet-500/15 text-violet-600 dark:text-violet-400",
  pink: "bg-pink-500/15 text-pink-600 dark:text-pink-400",
};

/** The swatch for picking a color. */
export const ICON_SWATCH_CLASS: Record<IconColor, string> = {
  gray: "bg-zinc-500",
  red: "bg-red-500",
  orange: "bg-orange-500",
  amber: "bg-amber-500",
  green: "bg-emerald-500",
  teal: "bg-teal-500",
  blue: "bg-blue-500",
  violet: "bg-violet-500",
  pink: "bg-pink-500",
};

export const ICON_COLORS = Object.keys(ICON_SWATCH_CLASS) as IconColor[];

/** "Ferret Media" → "FM", "podcast" → "P": a monogram's letters when none were typed. */
export function monogram(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words.slice(0, 2).map((word) => [...word][0]) : [...(words[0] ?? "?")].slice(0, 1);
  return letters.join("").toUpperCase() || "?";
}

export function iconText(icon: CollectionIcon, name: string): string {
  return icon.kind === "monogram" ? icon.text.trim() || monogram(name) : "";
}
