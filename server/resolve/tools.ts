// Linker-Edit's own Resolve tools, which agents get in the "linker" MCP server. Each runs a script from
// server/resolve/scripts through Resolve's own MCP server. The rules are in the scripts, not only in the
// prompt: they change only "AI - " timelines, and nothing here deletes media, bins or projects or opens
// another project.

/**
 * read: looks only. view: changes what Resolve shows, not the project, so it needs no OK and isn't an
 * edit. write: changes the project; waits for the user's OK in "ask" mode and is listed in the history.
 */
export type ResolveToolKind = "read" | "view" | "write";

export interface ResolveTool {
  name: string;
  kind: ResolveToolKind;
  description: string;
  inputSchema: { type: "object"; properties: Record<string, unknown>; required?: string[] };
  /** Seconds the script may run, at most 60. */
  timeout?: number;
}

const timelineRef = {
  type: "string",
  description: "The timeline's id or exact name. Leave out for the timeline open in Resolve.",
};
const itemIds = { type: "array", items: { type: "string" }, description: "Timeline item ids, as get_timeline lists them." };
const mediaIds = { type: "array", items: { type: "string" }, description: "Media pool item ids, as list_media or get_timeline list them." };

const tools: ResolveTool[] = [
  {
    name: "resolve_context",
    kind: "read",
    description: "What Resolve has open: its version and page, the project, its database and the current timeline with its frame rate and size.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "list_timelines",
    kind: "read",
    description:
      'Every timeline in the open project: id, name, frame rate, size, first and last frame, track counts, which one is current, and "ai": true for timelines agents made (named "AI - …"), the only ones Linker-Edit\'s tools change.',
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "get_timeline",
    kind: "read",
    description:
      'A timeline\'s clips per track and its markers. Each clip has id, name, start and end (timeline frames, end exclusive), source_start and source_end (frames in its media) and media (a media pool id; "media" at the top maps ids to name, file path and fps). Marker frames count from the timeline\'s first frame. Narrow long timelines with track_type, track and a frame range.',
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        track_type: { type: "string", enum: ["video", "audio"], description: "Only video or only audio tracks." },
        track: { type: "integer", description: "Only this track number, starting at 1." },
        from_frame: { type: "integer", description: "Only clips that end at or after this timeline frame." },
        to_frame: { type: "integer", description: "Only clips that start at or before this timeline frame." },
        properties: { type: "boolean", description: "Also return every clip's properties (transform, crop, opacity, speed…). Long." },
      },
    },
  },
  {
    name: "list_media",
    kind: "read",
    description:
      'The media pool: every bin\'s path ("Master/Interviews") and the clips with id, name, bin, type, file path, fps, frames, duration, resolution, start timecode and audio channels.',
    inputSchema: {
      type: "object",
      properties: {
        bin: { type: "string", description: 'Only this bin and the bins inside it, by path ("Master/Interviews") or name.' },
        search: { type: "string", description: "Only clips whose name or file path contains this, ignoring case." },
        limit: { type: "integer", description: "At most this many clips. Defaults to 200." },
      },
    },
  },
  {
    name: "get_clip_properties",
    kind: "read",
    description: "Media pool clips' properties as Resolve names them (\"FPS\", \"File Path\", \"Duration\", \"Video Codec\", \"Comments\"…).",
    inputSchema: {
      type: "object",
      properties: { media_ids: mediaIds, properties: { type: "array", items: { type: "string" }, description: "Only these properties." } },
      required: ["media_ids"],
    },
  },
  {
    name: "get_transcription",
    kind: "read",
    description:
      "The transcript Resolve keeps on a media pool clip (made by transcribe_in_resolve or in Resolve), with timecodes. Says so while Resolve is still transcribing.",
    inputSchema: {
      type: "object",
      properties: { media_id: { type: "string" }, words: { type: "boolean", description: "Also return each word's timecodes. Long." } },
      required: ["media_id"],
    },
  },
  {
    name: "set_current_timeline",
    kind: "view",
    description: "Opens a timeline in Resolve, so the user sees it. Changes nothing in it.",
    inputSchema: { type: "object", properties: { timeline: timelineRef }, required: ["timeline"] },
  },
  {
    name: "create_timeline",
    kind: "write",
    description:
      'Makes a new, empty timeline named "AI - <name>" (a number is added when the name is taken) and opens it. Leave out the size and frame rate to use the project\'s.',
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: 'Short description; "AI - " is added.' },
        width: { type: "integer" },
        height: { type: "integer" },
        frame_rate: { type: "number", description: "Only settable on a new, empty timeline." },
        start_timecode: { type: "string", description: 'Like "01:00:00:00".' },
      },
      required: ["name"],
    },
  },
  {
    name: "duplicate_timeline",
    kind: "write",
    description:
      'Copies a timeline to a new one named "AI - <name>" and opens the copy. To change a timeline that isn\'t an AI timeline, copy it with this and work on the copy.',
    inputSchema: {
      type: "object",
      properties: { timeline: timelineRef, name: { type: "string", description: 'Defaults to "<timeline> copy"; "AI - " is added.' } },
    },
  },
  {
    name: "add_clips",
    kind: "write",
    description:
      'Puts media pool clips on an AI timeline and opens it. Each clip takes the fields get_timeline returns, so a saved timeline\'s rows from the edit history (cuts.json, audio.json) can be passed back as they are. Without start a clip goes after the last one.',
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        clips: {
          type: "array",
          items: {
            type: "object",
            properties: {
              media: { type: "string", description: "Media pool item id." },
              source_start: { type: "number", description: "First frame of the media to use." },
              source_end: { type: "number", description: "End frame in the media, as get_timeline reports it." },
              start: { type: "number", description: "Timeline frame to place it at (timeline frames, like get_timeline's start)." },
              track: { type: "integer", description: "Track number, starting at 1. Missing tracks are added." },
              type: { type: "string", enum: ["video", "audio", "both"], description: 'Which part of the clip. Defaults to "both".' },
            },
            required: ["media"],
          },
        },
      },
      required: ["clips"],
    },
    timeout: 60,
  },
  {
    name: "remove_clips",
    kind: "write",
    description: "Takes clips off an AI timeline. Their media stays in the media pool.",
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        item_ids: itemIds,
        ripple: { type: "boolean", description: "Close the gaps they leave." },
      },
      required: ["item_ids"],
    },
  },
  {
    name: "set_clip_properties",
    kind: "write",
    description:
      'Sets clip properties on an AI timeline\'s clips, by Resolve\'s names: Pan, Tilt, ZoomX, ZoomY, Opacity, RotationAngle, CropLeft/Right/Top/Bottom, FlipX, FlipY, CompositeMode, Speed, Volume and more (get_scripting_docs section "Looking up Timeline item properties"). enabled turns clips on or off.',
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        item_ids: itemIds,
        properties: { type: "object", description: 'Like {"ZoomX": 1.2, "ZoomY": 1.2}.' },
        enabled: { type: "boolean" },
      },
      required: ["item_ids"],
    },
  },
  {
    name: "add_markers",
    kind: "write",
    description: "Adds markers to an AI timeline. Frames count from the timeline's first frame, as get_timeline reports markers.",
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        markers: {
          type: "array",
          items: {
            type: "object",
            properties: {
              frame: { type: "integer" },
              color: { type: "string", description: 'Resolve\'s marker colors: "Blue", "Cyan", "Green", "Yellow", "Red", "Pink", "Purple", "Fuchsia", "Rose", "Lavender", "Sky", "Mint", "Lemon", "Sand", "Cocoa", "Cream". Defaults to Blue.' },
              name: { type: "string" },
              note: { type: "string" },
              duration: { type: "integer", description: "In frames. Defaults to 1." },
            },
            required: ["frame"],
          },
        },
      },
      required: ["markers"],
    },
  },
  {
    name: "import_media",
    kind: "write",
    description: 'Imports files on this computer into a media pool bin, made when missing ("AI imports" by default). Returns the new clips\' ids.',
    inputSchema: {
      type: "object",
      properties: {
        paths: { type: "array", items: { type: "string" }, description: "Absolute file paths." },
        bin: { type: "string", description: 'Bin path under the media pool\'s root, like "AI imports/Music".' },
      },
      required: ["paths"],
    },
    timeout: 60,
  },
  {
    name: "transcribe_in_resolve",
    kind: "write",
    description:
      "Starts Resolve's own transcription of media pool clips. Resolve stores the transcript on each clip, where the user sees it and can edit by text. Only when the user asks for transcripts in Resolve; the linker transcribe tool is faster for reading what is said.",
    inputSchema: {
      type: "object",
      properties: { media_ids: mediaIds, speakers: { type: "boolean", description: "Detect speakers too." } },
      required: ["media_ids"],
    },
  },
];

export const RESOLVE_TOOLS: readonly ResolveTool[] = tools;

export function findResolveTool(name: string): ResolveTool | undefined {
  return tools.find((tool) => tool.name === name);
}
