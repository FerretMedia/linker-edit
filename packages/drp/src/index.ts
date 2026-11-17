// @linker-edit/drp: reads and patches DaVinci Resolve .drp project and .drt timeline files without Resolve.
export { readDrp, mediaItems } from "./read.ts";
export type { DrpFile, Project, MediaFolder, MediaItem, Timeline, Track, TimelineClip } from "./read.ts";
export { checkVersion } from "./version.ts";
export type { VersionCheck } from "./version.ts";
export { patchDrp } from "./patch.ts";
export type { Edit } from "./patch.ts";
