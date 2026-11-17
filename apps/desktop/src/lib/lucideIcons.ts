// Lucide icons by name, for project icons; loaded as one chunk, and only once a project uses one. As in T3 Code.

import type { Component } from "svelte";

type Lucide = typeof import("@lucide/svelte");

let loading: Promise<Lucide> | null = null;

export const loadLucide = (): Promise<Lucide> => (loading ??= import("@lucide/svelte"));

const toPascal = (name: string) =>
  name
    .split("-")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");

const toKebab = (pascal: string) => (pascal.match(/[A-Z][a-z]*|[0-9]+[a-z]*/g) ?? []).join("-").toLowerCase();

let iconSet: Set<unknown> | null = null;

/** Older names (code-2) resolve too, through Lucide's aliases. */
export function iconByName(lucide: Lucide, name: string): Component<{ class?: string }> | undefined {
  const icon = (lucide as unknown as Record<string, unknown>)[toPascal(name)];
  iconSet ??= new Set(Object.values(lucide.icons));
  return iconSet.has(icon) ? (icon as Component<{ class?: string }>) : undefined;
}

let names: string[] | null = null;

export function iconNames(lucide: Lucide): string[] {
  return (names ??= Object.keys(lucide.icons).map(toKebab));
}
