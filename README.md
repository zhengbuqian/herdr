# Herdr fork

Personal client customizations for [Herdr](https://github.com/herdrdev/herdr), based on official `v0.9.3`:

- Group shared projects by name across Local and SSH machines.
- Place single-machine projects below shared projects, separated by a divider.
- Show checkouts directly under each machine, with tree markers and the main repo's branch first.
- Highlight the machine row belonging to the selected checkout's project.
- Nest renamed home workspaces under `~`, always first under each machine.
- Recreate a missing home shell while the custom client is connected.
- Open workspace context menus directly across machines with one right-click.
- Hide online-machine dots while keeping offline and error indicators.
- Remember grouping and collapsed sections across client launches.
- Detach idle clients after a configurable timeout, keeping panes and agents running.

All customizations are client-side; the server stays official.

## Project-oriented Spaces

In **Settings → Spaces**, choose **By name** to group workspaces with the same name across the local machine and saved SSH machines. Projects present on multiple machines show a project heading, then each machine and its checkouts as direct sibling tree rows. The main repository is always first and shows only its branch row, followed by one row per worktree. Projects present on only one machine appear together under that machine's heading using the regular workspace/worktree layout. Home (`~`) always appears under its machine, even when other machines also have a home workspace.

Shared projects always appear first, with a horizontal divider before the machine-organized section. Home (`~`) is the first workspace under each machine. The highlighted machine row belongs to the selected workspace's group: selecting a checkout in a shared project highlights that project's machine row.

Non-project workspaces whose panes start in the home directory are grouped under that machine's `~`, including renamed workspaces. Their labels are preserved. If there is no actual `~` workspace, a client-only home heading holds them; its children can be collapsed and expanded.

Each machine always has a `~` heading. While this custom client is running and connected, it recreates a missing real home shell workspace through the existing JSON API. The client embeds a one-shot Python 3 helper and uses SSH for saved remote machines; a per-user lock coordinates simultaneous clients. No server changes or resident helper are needed, and creating `~` keeps the current focus. Remote machines need Python 3. Closing all custom clients stops automatic home-workspace maintenance.

Right-clicking a workspace on another machine selects it and opens its context menu directly. Rename, close, and worktree actions then apply to that machine's workspace.

![Spaces grouped by project across machines](https://github.com/user-attachments/assets/762d0664-d019-4b95-ad94-9bb5eddca3b8)

Grouping and collapsed sections are saved as client preferences. Workspaces and their processes stay on their original machines, and compatible remote servers need no update for this view.

## Automatic client detach when idle

Set a timeout in `config.toml` on the machine running the client process:

```toml
[ui]
idle_detach_minutes = 30
```

The client detaches after the configured period without local keyboard, mouse, paste, focus, or terminal resize activity. Agent output and server updates do not reset the timer. Detaching leaves the server, panes, and agents running; launch Herdr again to reconnect. The default is `0`, which disables automatic detach.

Related discussion: [herdrdev/herdr#4556](https://github.com/herdrdev/herdr/discussions/4556).
