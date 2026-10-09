use super::project_spaces::{self, ProjectSpaceRow};
use super::render::{display_width, put_right_text, put_text, ShellRenderState};
use super::*;

fn workspace_rows(
    workspace: &ClientShellWorkspace,
    status: crate::api::schema::AgentStatus,
    shared_project: bool,
    entry: &WorkspaceEntry,
    config: &ClientShellConfig,
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    let linked = workspace
        .worktree
        .as_ref()
        .is_some_and(|tree| tree.is_linked_worktree);
    if shared_project && !linked && (workspace.worktree.is_some() || workspace.branch.is_some()) {
        super::sidebar::workspace_branch_rows(workspace, status, &config.spaces)
    } else {
        super::sidebar::workspace_rows(
            workspace,
            status,
            entry.indented || shared_project && linked,
            &config.spaces,
        )
    }
}

pub(super) fn render_expanded(
    buffer: &mut Buffer,
    area: Rect,
    active_snapshot: Option<&ClientShellSnapshot>,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    super::render::render_sidebar_background(buffer, area, palette);
    hits.sidebar_divider = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
    let (workspace_area, detail_area) =
        crate::ui::expanded_sidebar_sections(area, state.sidebar_section_split);
    hits.sidebar_section_divider =
        crate::ui::sidebar_section_divider_rect(area, state.sidebar_section_split);
    put_text(
        buffer,
        workspace_area.x,
        workspace_area.y,
        workspace_area.width,
        " projects",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );

    let mut rows = project_spaces::rows(
        state.endpoints,
        state.collapsed_projects,
        state.collapsed_project_machines,
    );
    let highlighted_workspace = state
        .selected_workspace_id
        .map(|target| (&target.endpoint_id, target.workspace_id.as_str()))
        .or_else(|| {
            active_snapshot
                .and_then(|snapshot| snapshot.focused_workspace_id.as_deref())
                .map(|workspace_id| (state.active_endpoint_id, workspace_id))
        });
    let highlighted_machine = highlighted_workspace.and_then(|(endpoint_id, workspace_id)| {
        let endpoint = state
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)?;
        let project = project_spaces::endpoint_workspace_project_name(endpoint, workspace_id)?;
        let shared = rows
            .iter()
            .any(|row| matches!(row, ProjectSpaceRow::Project { key, .. } if key == project));
        Some((endpoint_id, shared.then(|| project.to_owned())))
    });
    let empty_collapsed_groups = HashSet::new();
    rows.retain(|row| {
        let ProjectSpaceRow::Workspace {
            project_key: None,
            endpoint,
            entry,
        } = row
        else {
            return true;
        };
        if !entry.indented {
            return true;
        }
        let endpoint = &state.endpoints[*endpoint];
        let groups = if endpoint.endpoint_id.is_local() {
            state.collapsed_groups
        } else {
            state
                .remote_collapsed_groups
                .get(&endpoint.endpoint_id)
                .unwrap_or(&empty_collapsed_groups)
        };
        !endpoint
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.workspaces.get(entry.index))
            .and_then(|workspace| workspace.worktree.as_ref())
            .is_some_and(|tree| groups.contains(&tree.key))
    });
    let body = Rect::new(
        workspace_area.x,
        workspace_area.y.saturating_add(WORKSPACE_HEADER_ROWS),
        workspace_area.width,
        workspace_area
            .height
            .saturating_sub(WORKSPACE_HEADER_ROWS + 1),
    );
    hits.workspace_body = body;
    let row_heights = rows
        .iter()
        .map(|row| match row {
            ProjectSpaceRow::Workspace {
                project_key,
                endpoint,
                entry,
            } => state.endpoints[*endpoint]
                .snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.workspaces.get(entry.index))
                .map_or(1, |workspace| {
                    workspace_rows(
                        workspace,
                        workspace.agent_status,
                        project_key.is_some(),
                        entry,
                        config,
                    )
                    .len()
                    .max(1)
                    .min(u16::MAX as usize) as u16
                }),
            _ => 1,
        })
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            u16::from(matches!(
                (row, rows.get(index + 1)),
                (
                    ProjectSpaceRow::Workspace { endpoint, .. },
                    Some(ProjectSpaceRow::Workspace { endpoint: next, entry, .. })
                ) if endpoint == next && !entry.indented
            )) * config.spaces.row_gap
        })
        .collect::<Vec<_>>();
    let reveal_navigation = !body.is_empty() && std::mem::take(state.reveal_navigation_workspace);
    let reveal_focus = !body.is_empty() && std::mem::take(state.reveal_focused_workspace);
    if reveal_navigation || reveal_focus {
        let target_row = rows.iter().position(|row| {
            let (endpoint, index) = match row {
                ProjectSpaceRow::Workspace {
                    endpoint, entry, ..
                } => (*endpoint, entry.index),
                ProjectSpaceRow::Home {
                    endpoint,
                    parent: Some(index),
                    ..
                } => (*endpoint, *index),
                _ => return false,
            };
            let endpoint = &state.endpoints[endpoint];
            let Some(workspace) = endpoint
                .snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.workspaces.get(index))
            else {
                return false;
            };
            if reveal_navigation {
                state.selected_workspace_id.is_some_and(|target| {
                    target.matches(&endpoint.endpoint_id, &workspace.workspace_id)
                })
            } else {
                &endpoint.endpoint_id == state.active_endpoint_id && workspace.focused
            }
        });
        if let Some(target_row) = target_row {
            *state.workspace_scroll = super::scroll::list_scroll_start_to_reveal(
                &row_heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
                target_row,
            );
        }
    }
    let metrics = super::scroll::list_scroll_metrics(
        &row_heights,
        &gaps,
        body.height,
        *state.workspace_scroll,
    );
    hits.workspace_max_scroll = metrics.max_offset_from_bottom;
    hits.workspace_scroll_metrics = Some(metrics);
    *state.workspace_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (row_index, row) in rows.iter().enumerate().skip(*state.workspace_scroll) {
        if y >= body.bottom() {
            break;
        }
        let row_height = row_heights[row_index].min(body.height);
        if y.saturating_add(row_height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, row_height);
        match row {
            ProjectSpaceRow::Home {
                endpoint,
                parent,
                count,
                collapsed,
            } => {
                let endpoint = &state.endpoints[*endpoint];
                let nested = Rect::new(
                    rect.x.saturating_add(2),
                    rect.y,
                    rect.width.saturating_sub(2),
                    1,
                );
                let marker = if *collapsed { "▸" } else { "▾" };
                if let Some(workspace) =
                    parent.and_then(|index| endpoint.snapshot.as_deref()?.workspaces.get(index))
                {
                    let selected = state.selected_workspace_id.is_some_and(|target| {
                        target.matches(&endpoint.endpoint_id, &workspace.workspace_id)
                    });
                    let entry = WorkspaceEntry {
                        index: parent.unwrap_or_default(),
                        indented: false,
                        last_child: false,
                    };
                    let tokens = super::sidebar::workspace_rows(
                        workspace,
                        workspace.agent_status,
                        false,
                        &config.spaces,
                    );
                    super::sidebar::render_workspace_rows(
                        buffer,
                        nested,
                        workspace.agent_status,
                        config.status_indicators,
                        &entry,
                        tokens,
                        &endpoint.endpoint_id == state.active_endpoint_id && workspace.focused,
                        selected,
                        state.selected_workspace_id.is_some(),
                        false,
                        palette,
                    );
                    let toggle = Rect::new(
                        rect.right().saturating_sub(1),
                        rect.y,
                        u16::from(rect.width > 0),
                        1,
                    );
                    put_text(
                        buffer,
                        toggle.x,
                        toggle.y,
                        toggle.width,
                        marker,
                        Style::default().fg(palette.overlay1),
                    );
                    hits.workspaces.push(WorkspaceHit {
                        rect,
                        endpoint_id: endpoint.endpoint_id.clone(),
                        workspace_id: workspace.workspace_id.clone(),
                        indented: false,
                        group_toggle: Some((toggle, project_spaces::HOME_GROUP_KEY.to_owned())),
                    });
                } else {
                    put_text(
                        buffer,
                        nested.x,
                        nested.y,
                        nested.width,
                        &format!(" {marker} ~"),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    );
                    put_right_text(
                        buffer,
                        rect,
                        rect.y,
                        &count.to_string(),
                        Style::default().fg(palette.overlay0),
                    );
                    hits.machines.push(MachineHit {
                        rect,
                        status_badge: Rect::default(),
                        collapse_toggle: rect,
                        endpoint_id: endpoint.endpoint_id.clone(),
                        project_key: Some(project_spaces::HOME_GROUP_KEY.to_owned()),
                    });
                }
            }
            ProjectSpaceRow::Divider => {
                let width = rect.width.saturating_sub(2);
                put_text(
                    buffer,
                    rect.x.saturating_add(1),
                    rect.y,
                    width,
                    &"─".repeat(usize::from(width)),
                    Style::default().fg(palette.overlay1),
                );
            }
            ProjectSpaceRow::Project {
                key,
                label,
                count,
                collapsed,
            } => {
                let marker = if *collapsed { "▸" } else { "▾" };
                put_text(
                    buffer,
                    rect.x.saturating_add(1),
                    rect.y,
                    rect.width.saturating_sub(2),
                    &format!("{marker} {label}"),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                );
                put_right_text(
                    buffer,
                    rect,
                    rect.y,
                    &count.to_string(),
                    Style::default().fg(palette.overlay0),
                );
                hits.projects.push(project_spaces::ProjectHit {
                    rect,
                    key: key.clone(),
                });
            }
            ProjectSpaceRow::Machine {
                project_key,
                endpoint,
                count,
                collapsed,
            } => {
                let endpoint = &state.endpoints[*endpoint];
                let marker = if *collapsed { "▸" } else { "▾" };
                let indent = if project_key.is_some() { 2 } else { 1 };
                let badge = super::endpoint_sidebar::render_endpoint_row(
                    buffer,
                    Rect::new(
                        rect.x.saturating_add(indent),
                        rect.y,
                        rect.width.saturating_sub(indent),
                        1,
                    ),
                    marker,
                    endpoint,
                    highlighted_machine
                        .as_ref()
                        .is_some_and(|(endpoint_id, group)| {
                            *endpoint_id == &endpoint.endpoint_id && group == project_key
                        }),
                    state.machine_diagnostics,
                    palette,
                );
                if *count == 0 {
                    put_right_text(
                        buffer,
                        rect,
                        rect.y,
                        "0",
                        Style::default().fg(palette.overlay0),
                    );
                }
                hits.machines.push(MachineHit {
                    rect,
                    status_badge: badge,
                    collapse_toggle: Rect::new(
                        rect.x.saturating_add(indent + 1),
                        rect.y,
                        u16::from(rect.width > 3),
                        1,
                    ),
                    endpoint_id: endpoint.endpoint_id.clone(),
                    project_key: project_key.clone(),
                });
            }
            ProjectSpaceRow::Workspace {
                project_key,
                endpoint,
                entry,
            } => {
                let endpoint = &state.endpoints[*endpoint];
                let Some(snapshot) = endpoint.snapshot.as_deref() else {
                    continue;
                };
                let Some(workspace) = snapshot.workspaces.get(entry.index) else {
                    continue;
                };
                let collapsed_groups = if endpoint.endpoint_id.is_local() {
                    state.collapsed_groups
                } else {
                    state
                        .remote_collapsed_groups
                        .get(&endpoint.endpoint_id)
                        .unwrap_or(&empty_collapsed_groups)
                };
                let status = if project_key.is_some() {
                    workspace.agent_status
                } else {
                    super::sidebar::displayed_workspace_status(
                        snapshot,
                        workspace,
                        collapsed_groups,
                    )
                };
                let selected = state.selected_workspace_id.is_some_and(|target| {
                    target.matches(&endpoint.endpoint_id, &workspace.workspace_id)
                });
                let tokens =
                    workspace_rows(workspace, status, project_key.is_some(), entry, config);
                super::sidebar::render_workspace_rows(
                    buffer,
                    Rect::new(
                        rect.x.saturating_add(2),
                        rect.y,
                        rect.width.saturating_sub(2),
                        rect.height,
                    ),
                    status,
                    config.status_indicators,
                    entry,
                    tokens,
                    &endpoint.endpoint_id == state.active_endpoint_id && workspace.focused,
                    selected,
                    state.selected_workspace_id.is_some(),
                    false,
                    palette,
                );
                if endpoint.status != ClientEndpointStatus::Online {
                    buffer.set_style(
                        rect,
                        Style::default()
                            .fg(palette.overlay0)
                            .add_modifier(Modifier::DIM),
                    );
                }
                let group_toggle = project_key
                    .is_none()
                    .then(|| {
                        super::sidebar::render_parent_group_toggle(
                            buffer,
                            rect,
                            snapshot,
                            entry.index,
                            collapsed_groups,
                            palette,
                        )
                    })
                    .flatten();
                hits.workspaces.push(WorkspaceHit {
                    rect,
                    endpoint_id: endpoint.endpoint_id.clone(),
                    workspace_id: workspace.workspace_id.clone(),
                    indented: entry.indented,
                    group_toggle,
                });
            }
        }
        y = y.saturating_add(row_height).saturating_add(gaps[row_index]);
    }
    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, palette);
    }

    let footer_y = workspace_area.bottom().saturating_sub(1);
    if config.mouse_capture {
        let label = format!(
            " new · {}",
            super::endpoint_sidebar::active_endpoint_label(state)
        );
        hits.new_workspace = Rect::new(
            workspace_area.x,
            footer_y,
            display_width(&label).min(workspace_area.width),
            u16::from(workspace_area.height > 0),
        );
        put_text(
            buffer,
            workspace_area.x,
            footer_y,
            workspace_area.width,
            &label,
            Style::default().fg(palette.overlay0),
        );
        let attention = active_snapshot.is_some_and(super::global_menu::global_menu_attention);
        let width = if attention { 8 } else { 6 }.min(workspace_area.width);
        hits.global_launcher = Rect::new(
            workspace_area.right().saturating_sub(width),
            footer_y,
            width,
            1,
        );
        put_right_text(
            buffer,
            workspace_area,
            footer_y,
            if attention { "● menu" } else { "menu" },
            Style::default().fg(if attention {
                palette.accent
            } else {
                palette.overlay0
            }),
        );
    }
    super::endpoint_agents::render_expanded(
        buffer,
        detail_area,
        active_snapshot.and_then(|snapshot| snapshot.agent_view_label.as_deref()),
        state.endpoints,
        state.active_endpoint_id,
        config,
        state.agent_scroll,
        hits,
    );
    hits.sidebar_toggle = Rect::new(
        area.right().saturating_sub(2),
        area.bottom().saturating_sub(1),
        u16::from(area.width > 1),
        u16::from(area.height > 0),
    );
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "«",
        Style::default().fg(palette.overlay0),
    );
}

pub(super) fn render_collapsed(
    buffer: &mut Buffer,
    area: Rect,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    super::render::render_sidebar_background(buffer, area, palette);
    let (workspace_area, divider_y, detail_area) = super::sidebar::collapsed_sidebar_sections(area);
    let rows = project_spaces::rows(state.endpoints, &HashSet::new(), &HashSet::new());
    let workspaces = project_spaces::workspace_order(&rows);
    let height = usize::from(workspace_area.height);
    let max_scroll = workspaces.len().saturating_sub(height);
    *state.workspace_scroll = (*state.workspace_scroll).min(max_scroll);
    let reveal_navigation = std::mem::take(state.reveal_navigation_workspace);
    let reveal_focus = std::mem::take(state.reveal_focused_workspace);
    if reveal_navigation || reveal_focus {
        let target = workspaces.iter().position(|(endpoint_index, index)| {
            let endpoint = &state.endpoints[*endpoint_index];
            let Some(workspace) = endpoint
                .snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.workspaces.get(*index))
            else {
                return false;
            };
            if reveal_navigation {
                state.selected_workspace_id.is_some_and(|target| {
                    target.matches(&endpoint.endpoint_id, &workspace.workspace_id)
                })
            } else {
                &endpoint.endpoint_id == state.active_endpoint_id && workspace.focused
            }
        });
        if let Some(target) = target {
            if target < *state.workspace_scroll {
                *state.workspace_scroll = target;
            } else if target >= state.workspace_scroll.saturating_add(height) {
                *state.workspace_scroll = target
                    .saturating_add(1)
                    .saturating_sub(height)
                    .min(max_scroll);
            }
        }
    }
    hits.workspace_max_scroll = max_scroll;
    for (position, (endpoint_index, index)) in workspaces
        .iter()
        .enumerate()
        .skip(*state.workspace_scroll)
        .take(height)
    {
        let endpoint = &state.endpoints[*endpoint_index];
        let Some(workspace) = endpoint
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.workspaces.get(*index))
        else {
            continue;
        };
        let rect = Rect::new(
            workspace_area.x,
            workspace_area.y + (position - *state.workspace_scroll) as u16,
            workspace_area.width,
            1,
        );
        let selected = state
            .selected_workspace_id
            .is_some_and(|target| target.matches(&endpoint.endpoint_id, &workspace.workspace_id));
        let focused = &endpoint.endpoint_id == state.active_endpoint_id && workspace.focused;
        if selected || focused {
            let selection_background = if palette.selection_bg == ratatui::style::Color::Reset {
                palette.active_row_bg
            } else {
                palette.selection_bg
            };
            buffer.set_style(
                rect,
                Style::default().bg(if selected {
                    selection_background
                } else {
                    palette.active_row_bg
                }),
            );
        }
        let machine = if endpoint.endpoint_id.is_local() {
            "L".to_owned()
        } else {
            (endpoint_index + 1).to_string()
        };
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width.min(2),
            &machine,
            Style::default().fg(palette.overlay0),
        );
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(workspace.agent_status, config.status_indicators),
            Style::default().fg(status_color(workspace.agent_status, palette)),
        );
        hits.workspaces.push(WorkspaceHit {
            rect,
            endpoint_id: endpoint.endpoint_id.clone(),
            workspace_id: workspace.workspace_id.clone(),
            indented: false,
            group_toggle: None,
        });
    }
    if let Some(divider_y) = divider_y {
        put_text(
            buffer,
            workspace_area.x,
            divider_y,
            workspace_area.width,
            &"─".repeat(workspace_area.width as usize),
            Style::default().fg(palette.surface_dim),
        );
    }
    super::endpoint_agents::render_collapsed(
        buffer,
        detail_area,
        state.endpoints,
        state.active_endpoint_id,
        config,
        hits,
    );
    hits.sidebar_toggle = if area.is_empty() || workspace_area.width == 0 {
        Rect::default()
    } else {
        Rect::new(
            workspace_area.x + workspace_area.width / 2,
            area.bottom().saturating_sub(1),
            1,
            1,
        )
    };
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "»",
        Style::default().fg(palette.overlay0),
    );
}
