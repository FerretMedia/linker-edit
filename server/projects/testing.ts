import type { ProjectStore, SavedProject, SavedThread } from "./store.ts";

/** A ProjectStore in memory, for app tests: what it saves is what the next app loads. */
export function memoryProjectStore(initial: { project: SavedProject; threads?: SavedThread[] }[] = []) {
  const projects = new Map(initial.map(({ project }) => [project.id, structuredClone(project)]));
  const threads = new Map<string, Map<string, SavedThread>>(
    initial.map(({ project, threads = [] }) => [project.id, new Map(threads.map((thread) => [thread.id, structuredClone(thread)]))]),
  );
  let next = 0;
  const store: ProjectStore & { projects: Map<string, SavedProject>; threads: Map<string, Map<string, SavedThread>> } = {
    projects,
    threads,
    load: () =>
      [...projects.values()].map((project) => ({
        project: structuredClone(project),
        threads: [...(threads.get(project.id)?.values() ?? [])].map((thread) => structuredClone(thread)),
      })),
    create(fields) {
      next += 1;
      const project: SavedProject = { ...structuredClone(fields), id: `project-${next}`, dir: `/projects/${next}` };
      projects.set(project.id, structuredClone(project));
      threads.set(project.id, new Map());
      return project;
    },
    adopt(dir) {
      throw new Error(`${dir} has no linker-edit.json, so it isn't a Linker-Edit project folder.`);
    },
    saveProject(project) {
      projects.set(project.id, structuredClone(project));
    },
    removeProject(id) {
      projects.delete(id);
    },
    saveThread(project, thread) {
      threads.get(project.id)?.set(thread.id, structuredClone(thread));
    },
    deleteThread(project, threadId) {
      threads.get(project.id)?.delete(threadId);
    },
  };
  return store;
}

export const workspaceProject: SavedProject = { id: "workspace", dir: "/work", name: null, link: null, local: {} };
