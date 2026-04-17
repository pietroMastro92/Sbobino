# Epic: Workspaces and smart folders

Suggested labels: `epic`, `product`, `organization`, `automation`

## Problem

As automatic ingestion grows, the artifact list can become noisy. Users need a durable organizational layer that maps incoming content to a meaningful context such as course, project, customer, or team.

## Goal

Group transcripts into workspaces and route new items into the right place automatically.

## User value

- Students can separate courses and subjects.
- Professionals can keep customers, projects, and internal meetings distinct.
- Automatic import remains manageable instead of becoming a flat inbox.

## Proposal

Add:

- `workspaceId` as a first-class artifact association
- mapping from watched folders or source presets to a workspace
- smart-folder style filters for source, workspace, status, and generated outputs
- optional automatic tags derived from source rules

## V1 scope

- local workspace metadata and filtering
- folder-to-workspace mapping
- basic automatic tags
- no shared cloud collaboration model in V1

## Acceptance criteria

- Users can create and rename workspaces.
- A watched source can route incoming items into a selected workspace.
- Artifact history can be filtered quickly by workspace and source.
- Automatic imports remain visible even when routed into specific workspaces.

## Test scenarios

- Files from a course folder land in the selected course workspace.
- Files from a client meeting folder land in the correct project workspace.
- Workspace filters do not hide unassigned imports unexpectedly.

## Out of scope

- Multi-user workspace permissions
- External project-management integrations
- Workspace-level billing or licensing
