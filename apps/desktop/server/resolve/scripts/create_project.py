# Saves the open project, then makes a new, empty one in the database Resolve has open, and opens it. ARGS: name.
# Only for the user's own click in Add project: agents may not make projects (see policy.ts).
name = str(ARGS.get("name") or "").strip()
if not name:
    raise Refused("Give the project a name.")
if not local_library():
    # Blackmagic Cloud's terms (31.5) don't allow scripts to use the service, and what it reports as its type isn't documented.
    raise Refused("Resolve doesn't have a local or network database open. Make the project in Resolve's Project Manager yourself, then add it.")
current = pm.GetCurrentProject()
taken = saved_somewhere(name) or (current and current.GetName() == name)
if taken:
    raise Refused(f'Resolve already has a project named "{name}". Pick another name, or add that one from Database.')
if current and not untouched(current):
    check_leaving(current)
    pm.SaveProject()
pm.GotoRootFolder()
made = pm.CreateProject(name)
if not made:
    raise Refused(f'Resolve couldn\'t make the project "{name}". Try another name, and close any dialog in Resolve.')
pm.SaveProject()
result = opened(made, [])
