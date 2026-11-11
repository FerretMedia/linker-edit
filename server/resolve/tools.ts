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

/**
 * The tools Claude always has loaded. The rest wait behind its tool search until a task needs them, so
 * they cost no context in every chat.
 */
export const CORE_TOOLS = new Set([
  "resolve_context",
  "list_timelines",
  "get_timeline",
  "list_media",
  "view_frames",
  "find_silences",
  "set_current_timeline",
  "create_timeline",
  "duplicate_timeline",
  "add_clips",
  "remove_clips",
  "set_clip_properties",
  "add_markers",
  "add_title",
  "render_timeline",
]);

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
  {
    name: "view_frames",
    kind: "read",
    description:
      "Shows you frames as images, to see what footage looks like or to check your edit. Either timeline frames (Resolve renders the exact picture, with color and effects, then puts the playhead back), or moments of a media file in seconds. At most 12 frames per call.",
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        frames: { type: "array", items: { type: "integer" }, description: "Timeline frames, like get_timeline's start." },
        file: { type: "string", description: "Instead of a timeline: an absolute media file path (list_media's file_path)." },
        seconds: { type: "array", items: { type: "number" }, description: "With file: moments in seconds from the file's start." },
      },
    },
    timeout: 60,
  },
  {
    name: "find_silences",
    kind: "read",
    description:
      'Finds the silent stretches in media files on this computer, from their audio, to cut silences and pauses. Returns per file its duration and the silent ranges in seconds from the file\'s start (source frame = seconds × the clip\'s fps), plus the "speech" ranges between them. Fast: no transcription.',
    inputSchema: {
      type: "object",
      properties: {
        files: { type: "array", items: { type: "string" }, description: "Absolute file paths (list_media's file_path)." },
        min_silence: { type: "number", description: "Shortest pause to report, in seconds. Defaults to 0.5." },
        threshold_db: { type: "number", description: "Quieter than this counts as silence, in dB. Defaults to -35; use -45 for quiet rooms, -25 for noisy ones." },
        padding: { type: "number", description: "Seconds of each silence kept next to speech, so cuts don't clip words. Defaults to 0.1." },
      },
      required: ["files"],
    },
  },
  {
    name: "export_timeline",
    kind: "read",
    description:
      'Writes a timeline to an interchange file in the project\'s working folder (exports/): OTIO, EDL, FCPXML, FCP 7 XML, AAF, CSV or DRT. Changes nothing in Resolve. An OTIO you edit can come back as a new timeline with import_timeline.',
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        format: { type: "string", enum: ["otio", "edl", "fcpxml", "fcp7xml", "aaf", "csv", "drt"], description: 'Defaults to "otio".' },
        name: { type: "string", description: "File name without extension. Defaults to the timeline's name." },
      },
    },
  },
  {
    name: "import_timeline",
    kind: "write",
    description:
      'Makes a new AI timeline from an OTIO, EDL, FCPXML, FCP 7 XML or AAF file, linked to clips already in the media pool, and opens it. Useful for edits easier to write as a file than clip by clip.',
    inputSchema: {
      type: "object",
      properties: {
        path: { type: "string", description: "The file: absolute, or relative to the working folder." },
        name: { type: "string", description: '"AI - " is added.' },
        import_media: { type: "boolean", description: "Also import the media files it names into the media pool. Off by default." },
      },
      required: ["path", "name"],
    },
    timeout: 60,
  },
  {
    name: "add_title",
    kind: "write",
    description:
      "Adds a Fusion title (Text+ by default) with your text to an AI timeline at a frame, on the video track Resolve picks there. It gets Resolve's standard title length; the result says where it went.",
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        text: { type: "string" },
        frame: { type: "integer", description: "Timeline frame to start at. Defaults to the playhead." },
        title: { type: "string", description: 'Another Fusion title by its name in Resolve\'s Effects library. Defaults to "Text+".' },
        size: { type: "number", description: "Text+ size, 0 to 0.5; Resolve's default is 0.08." },
        font: { type: "string" },
      },
      required: ["text"],
    },
  },
  {
    name: "set_tracks",
    kind: "write",
    description: "Names, turns on or off, and locks an AI timeline's tracks. Tracks that don't exist yet are added.",
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        tracks: {
          type: "array",
          items: {
            type: "object",
            properties: {
              type: { type: "string", enum: ["video", "audio", "subtitle"] },
              index: { type: "integer", description: "From 1." },
              name: { type: "string" },
              enabled: { type: "boolean" },
              locked: { type: "boolean" },
            },
            required: ["type", "index"],
          },
        },
      },
      required: ["tracks"],
    },
  },
  {
    name: "remove_markers",
    kind: "write",
    description: 'Removes markers from an AI timeline, by frame or by color ("All" for every one).',
    inputSchema: {
      type: "object",
      properties: { timeline: timelineRef, frames: { type: "array", items: { type: "integer" } }, color: { type: "string" } },
    },
  },
  {
    name: "set_color",
    kind: "write",
    description:
      "Grades an AI timeline's clips: a LUT or a CDL on a node of each, or copy_from another clip's grade onto them (to match shots).",
    inputSchema: {
      type: "object",
      properties: {
        timeline: timelineRef,
        item_ids: itemIds,
        lut: { type: "string", description: "LUT file: absolute, or relative to Resolve's LUT folder (Resolve's list_luts shows them)." },
        cdl: {
          type: "object",
          description: "Each of slope, offset and power is one number or [r, g, b].",
          properties: { slope: {}, offset: {}, power: {}, saturation: { type: "number" } },
        },
        node: { type: "integer", description: "Node number for lut and cdl. Defaults to 1." },
        copy_from: { type: "string", description: "Timeline item id whose grade to copy." },
      },
      required: ["item_ids"],
    },
  },
  {
    name: "organize_media",
    kind: "write",
    description:
      'Organizes media pool clips without deleting anything: moves them into a bin (made when missing), sets their clip color, adds a flag, and writes metadata such as "Comments", "Keywords", "Description", "Scene", "Shot" or "Take". Good for marking selects.',
    inputSchema: {
      type: "object",
      properties: {
        media_ids: mediaIds,
        bin: { type: "string", description: 'Bin path, like "Selects/Interviews".' },
        clip_color: { type: "string", description: '"Orange", "Apricot", "Yellow", "Lime", "Olive", "Green", "Teal", "Navy", "Blue", "Purple", "Violet", "Pink", "Tan", "Beige", "Brown", "Chocolate".' },
        flag: { type: "string", description: 'Flag color, like "Green" or "Red".' },
        metadata: { type: "object", description: 'Like {"Comments": "Best take", "Keywords": "interview, outdoor"}.' },
      },
      required: ["media_ids"],
    },
  },
  {
    name: "detect_scene_cuts",
    kind: "write",
    description: "Has Resolve cut an AI timeline's clips where the scene changes, to split long recordings into shots. Waits until Resolve is done.",
    inputSchema: { type: "object", properties: { timeline: timelineRef } },
  },
  {
    name: "render_options",
    kind: "read",
    description: "What render_timeline can use: Resolve's render presets, file formats, and the codecs of one format.",
    inputSchema: { type: "object", properties: { format: { type: "string", description: 'Format to list codecs for, like "mp4" or "mov".' } } },
  },
  {
    name: "render_timeline",
    kind: "write",
    description:
      "Renders a timeline to a file, by default an H.264 MP4 at the timeline's size in the working folder's renders/ folder, and waits until it's done. Puts Resolve's timeline, page and render settings back after. Use a preset, or format, codec and size, and optionally a frame range.",
    inputSchema: {
      type: "object",
      properties: {
        timeline: { type: "string", description: "The timeline's id or exact name. Leave out for the one open in Resolve." },
        preset: { type: "string", description: "One of Resolve's render presets (render_options)." },
        format: { type: "string", description: 'Like "mp4" or "mov".' },
        codec: { type: "string", description: 'Like "H264", "H265" or "ProRes422HQ" (render_options).' },
        width: { type: "integer" },
        height: { type: "integer" },
        from_frame: { type: "integer", description: "First timeline frame to render." },
        to_frame: { type: "integer", description: "Last timeline frame to render." },
        audio_only: { type: "boolean" },
        name: { type: "string", description: "File name without extension. Defaults to the timeline's name." },
        dir: { type: "string", description: "Absolute output folder. Defaults to renders/ in the working folder." },
      },
    },
  },
];

export const RESOLVE_TOOLS: readonly ResolveTool[] = tools;

export function findResolveTool(name: string): ResolveTool | undefined {
  return tools.find((tool) => tool.name === name);
}
