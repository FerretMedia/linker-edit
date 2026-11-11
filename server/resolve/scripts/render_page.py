# Opens the page Resolve showed before a render. ARGS: {"page": page name}
if resolve.GetCurrentPage() != ARGS["page"]:
    resolve.OpenPage(ARGS["page"])
result = {"page": resolve.GetCurrentPage()}
