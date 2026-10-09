use std::collections::{HashMap, HashSet};

use super::{
    ClientEndpointId, ClientShellEndpoint, ClientShellSnapshot, ClientShellWorkspace,
    WorkspaceEntry,
};

impl super::ClientShellState {
    pub(crate) fn missing_home_workspaces(&self) -> Vec<(ClientEndpointId, String)> {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.status == super::ClientEndpointStatus::Online)
            .filter_map(|endpoint| {
                let snapshot = endpoint.snapshot.as_deref()?;
                (!snapshot
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.label == "~"))
                .then(|| (endpoint.endpoint_id.clone(), snapshot.boot_id.clone()))
            })
            .collect()
    }
}

pub(super) const MACHINE_PROJECT_KEY: &str = "\0machine-spaces";
pub(super) const HOME_GROUP_KEY: &str = "\0home-spaces";

pub(super) struct ProjectHit {
    pub(super) rect: ratatui::layout::Rect,
    pub(super) key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProjectSpaceRow {
    Divider,
    Project {
        key: String,
        label: String,
        count: usize,
        collapsed: bool,
    },
    Machine {
        project_key: Option<String>,
        endpoint: usize,
        count: usize,
        collapsed: bool,
    },
    Home {
        endpoint: usize,
        parent: Option<usize>,
        count: usize,
        collapsed: bool,
    },
    Workspace {
        project_key: Option<String>,
        endpoint: usize,
        entry: WorkspaceEntry,
    },
}

struct ProjectGroup {
    key: String,
    label: String,
    members: Vec<Vec<usize>>,
}

fn is_home_directory(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    if path == "/root" || path == "~" {
        return true;
    }
    path.strip_prefix("/Users/")
        .or_else(|| path.strip_prefix("/home/"))
        .is_some_and(|user| !user.is_empty() && !user.contains('/'))
}

fn home_workspaces(snapshot: &ClientShellSnapshot) -> HashSet<&str> {
    let mut directories = HashMap::<&str, bool>::new();
    for pane in &snapshot.panes {
        if let Some(cwd) = pane.cwd.as_deref() {
            let home = is_home_directory(cwd);
            directories
                .entry(&pane.workspace_id)
                .and_modify(|all_home| *all_home &= home)
                .or_insert(home);
        }
    }
    snapshot
        .workspaces
        .iter()
        .filter(|workspace| {
            workspace.worktree.is_none()
                && workspace.branch.is_none()
                && directories
                    .get(workspace.workspace_id.as_str())
                    .copied()
                    .unwrap_or_else(|| is_home_directory(&workspace.new_workspace_cwd))
        })
        .map(|workspace| workspace.workspace_id.as_str())
        .collect()
}

fn root_workspace_names(snapshot: &ClientShellSnapshot) -> HashMap<&str, &str> {
    let mut names = HashMap::new();
    for workspace in &snapshot.workspaces {
        if let Some(worktree) = workspace
            .worktree
            .as_ref()
            .filter(|worktree| !worktree.is_linked_worktree)
        {
            names
                .entry(worktree.key.as_str())
                .or_insert(workspace.label.as_str());
        }
    }
    names
}

fn workspace_project_name<'a>(
    workspace: &'a ClientShellWorkspace,
    root_names: &HashMap<&str, &'a str>,
) -> &'a str {
    workspace
        .worktree
        .as_ref()
        .filter(|worktree| worktree.is_linked_worktree)
        .map(|worktree| {
            root_names
                .get(worktree.key.as_str())
                .copied()
                .unwrap_or(&worktree.label)
        })
        .filter(|name| !name.is_empty())
        .unwrap_or(&workspace.label)
}

pub(super) fn endpoint_workspace_project_name<'a>(
    endpoint: &'a ClientShellEndpoint,
    workspace_id: &str,
) -> Option<&'a str> {
    let snapshot = endpoint.snapshot.as_deref()?;
    let roots = root_workspace_names(snapshot);
    let homes = home_workspaces(snapshot);
    snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == workspace_id)
        .map(|workspace| {
            if homes.contains(workspace.workspace_id.as_str()) {
                "~"
            } else {
                workspace_project_name(workspace, &roots)
            }
        })
}

pub(super) fn rows(
    endpoints: &[ClientShellEndpoint],
    collapsed_projects: &HashSet<String>,
    collapsed_machines: &HashSet<(String, ClientEndpointId)>,
) -> Vec<ProjectSpaceRow> {
    let mut groups = HashMap::<String, ProjectGroup>::new();
    for (endpoint_index, endpoint) in endpoints.iter().enumerate() {
        let Some(snapshot) = endpoint.snapshot.as_deref() else {
            continue;
        };
        let roots = root_workspace_names(snapshot);
        let homes = home_workspaces(snapshot);
        for (index, workspace) in snapshot.workspaces.iter().enumerate() {
            let name = if homes.contains(workspace.workspace_id.as_str()) {
                "~"
            } else {
                workspace_project_name(workspace, &roots)
            };
            let group = group_for(&mut groups, name, name, endpoints.len());
            group.members[endpoint_index].push(index);
        }
    }

    let mut groups = groups.into_values().collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        left.label
            .to_lowercase()
            .cmp(&right.label.to_lowercase())
            .then_with(|| left.key.cmp(&right.key))
    });

    let mut rows = Vec::new();
    let mut machine_workspaces = endpoints
        .iter()
        .map(|endpoint| {
            vec![
                false;
                endpoint
                    .snapshot
                    .as_deref()
                    .map_or(0, |snapshot| snapshot.workspaces.len())
            ]
        })
        .collect::<Vec<_>>();
    for group in groups {
        let machine_count = group
            .members
            .iter()
            .filter(|members| !members.is_empty())
            .count();
        if machine_count < 2 || group.key == "~" {
            for (endpoint, members) in group.members.into_iter().enumerate() {
                for index in members {
                    machine_workspaces[endpoint][index] = true;
                }
            }
            continue;
        }
        let count = group.members.iter().map(Vec::len).sum();
        let collapsed = collapsed_projects.contains(&group.key);
        rows.push(ProjectSpaceRow::Project {
            key: group.key.clone(),
            label: group.label,
            count,
            collapsed,
        });
        if collapsed {
            continue;
        }
        for (endpoint_index, mut members) in group.members.into_iter().enumerate() {
            if members.is_empty() {
                continue;
            }
            let endpoint = &endpoints[endpoint_index];
            let machine_collapsed =
                collapsed_machines.contains(&(group.key.clone(), endpoint.endpoint_id.clone()));
            rows.push(ProjectSpaceRow::Machine {
                project_key: Some(group.key.clone()),
                endpoint: endpoint_index,
                count: members.len(),
                collapsed: machine_collapsed,
            });
            if machine_collapsed {
                continue;
            }
            if let Some(snapshot) = endpoint.snapshot.as_deref() {
                members.sort_by_key(|index| {
                    let workspace = &snapshot.workspaces[*index];
                    workspace
                        .worktree
                        .as_ref()
                        .is_some_and(|worktree| worktree.is_linked_worktree)
                });
            }
            let member_count = members.len();
            rows.extend(members.into_iter().enumerate().map(|(position, index)| {
                ProjectSpaceRow::Workspace {
                    project_key: Some(group.key.clone()),
                    endpoint: endpoint_index,
                    entry: WorkspaceEntry {
                        index,
                        indented: true,
                        last_child: position + 1 == member_count,
                    },
                }
            }));
        }
    }
    let shared_section_end = rows.len();
    for (endpoint_index, owned) in machine_workspaces.iter().enumerate() {
        let endpoint = &endpoints[endpoint_index];
        let count = owned.iter().filter(|owned| **owned).count();
        let snapshot = endpoint.snapshot.as_deref();
        let collapsed = collapsed_machines
            .contains(&(MACHINE_PROJECT_KEY.to_owned(), endpoint.endpoint_id.clone()));
        if shared_section_end > 0 && rows.len() == shared_section_end {
            rows.push(ProjectSpaceRow::Divider);
        }
        rows.push(ProjectSpaceRow::Machine {
            project_key: None,
            endpoint: endpoint_index,
            count,
            collapsed,
        });
        if collapsed {
            continue;
        }
        if let Some(snapshot) = snapshot {
            let homes = home_workspaces(snapshot);
            let mut entries = super::sidebar::workspace_entries(snapshot, &HashSet::new());
            entries.retain(|entry| owned[entry.index]);
            let home_entries = entries
                .iter()
                .filter(|entry| {
                    homes.contains(snapshot.workspaces[entry.index].workspace_id.as_str())
                })
                .copied()
                .collect::<Vec<_>>();
            entries.retain(|entry| {
                !homes.contains(snapshot.workspaces[entry.index].workspace_id.as_str())
            });
            {
                let parent = home_entries
                    .iter()
                    .find(|entry| snapshot.workspaces[entry.index].label == "~")
                    .map(|entry| entry.index);
                let collapsed = collapsed_machines
                    .contains(&(HOME_GROUP_KEY.to_owned(), endpoint.endpoint_id.clone()));
                rows.push(ProjectSpaceRow::Home {
                    endpoint: endpoint_index,
                    parent,
                    count: home_entries.len(),
                    collapsed,
                });
                if !collapsed {
                    let children = home_entries
                        .into_iter()
                        .filter(|entry| Some(entry.index) != parent)
                        .collect::<Vec<_>>();
                    let child_count = children.len();
                    rows.extend(children.into_iter().enumerate().map(|(position, entry)| {
                        ProjectSpaceRow::Workspace {
                            project_key: None,
                            endpoint: endpoint_index,
                            entry: WorkspaceEntry {
                                index: entry.index,
                                indented: true,
                                last_child: position + 1 == child_count,
                            },
                        }
                    }));
                }
            }
            rows.extend(entries.into_iter().map(|entry| ProjectSpaceRow::Workspace {
                project_key: None,
                endpoint: endpoint_index,
                entry,
            }));
        } else {
            rows.push(ProjectSpaceRow::Home {
                endpoint: endpoint_index,
                parent: None,
                count: 0,
                collapsed: true,
            });
        }
    }
    rows
}

fn group_for<'a>(
    groups: &'a mut HashMap<String, ProjectGroup>,
    key: &str,
    label: &str,
    endpoint_count: usize,
) -> &'a mut ProjectGroup {
    groups
        .entry(key.to_owned())
        .or_insert_with(|| ProjectGroup {
            key: key.to_owned(),
            label: label.to_owned(),
            members: vec![Vec::new(); endpoint_count],
        })
}

pub(super) fn workspace_order(rows: &[ProjectSpaceRow]) -> Vec<(usize, usize)> {
    rows.iter()
        .filter_map(|row| match row {
            ProjectSpaceRow::Workspace {
                endpoint, entry, ..
            } => Some((*endpoint, entry.index)),
            ProjectSpaceRow::Home {
                endpoint,
                parent: Some(index),
                ..
            } => Some((*endpoint, *index)),
            _ => None,
        })
        .collect()
}
