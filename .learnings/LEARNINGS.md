## [LRN-20260325-001] correction

**Logged**: 2026-03-25T12:35:00+01:00
**Priority**: medium
**Status**: pending
**Area**: frontend

### Summary
Transcript-specific display controls belong in the right inspector, not in the left navigation sidebar.

### Details
The confidence-color toggle for transcript rendering was initially added to the left app sidebar. User feedback clarified that this control must live in the right inspector panel, next to transcript-specific controls, because it affects the currently open detail view rather than global navigation.

### Suggested Action
When adding transcript, segment, or detail-view options, prefer the right inspector unless the control changes app-level navigation or global layout.

### Metadata
- Source: user_feedback
- Related Files: sbobino_desktop/apps/desktop/src/App.tsx, sbobino_desktop/apps/desktop/src/styles.css
- Tags: inspector, sidebar, transcript, ux

---

## [LRN-20260325-003] correction

**Logged**: 2026-03-25T15:24:00Z
**Priority**: high
**Status**: pending
**Area**: frontend

### Summary
Tooltip pointers for animated overlays should not live inside the same element that is being clipped during the morph animation.

### Details
The transcript confidence tooltip still rendered without a visible arrow after multiple CSS tweaks because the pointer was implemented as a child of the bubble element while the bubble itself used `clip-path` for the genie animation. That made the arrow easy to clip away or misplace even when its own CSS looked correct. Moving the pointer to a sibling wrapper tied to the portal shell fixes the visibility and positioning problem.

### Suggested Action
For future tooltip or popover animations in sbobino, separate shell motion, bubble masking, and arrow rendering into different layers instead of animating everything inside one clipped node.

### Metadata
- Source: user_feedback
- Related Files: sbobino_desktop/apps/desktop/src/components/ConfidenceTranscript.tsx, sbobino_desktop/apps/desktop/src/styles.css
- Tags: tooltip, clip-path, portal, animation
- See Also: LRN-20260325-002

---

## [LRN-20260325-002] correction

**Logged**: 2026-03-25T15:17:41Z
**Priority**: medium
**Status**: pending
**Area**: frontend

### Summary
When polishing UI motion, user-visible artifacts like arrow direction and tinted borders matter more than subtle animation tweaks.

### Details
The tooltip animation for transcript confidence was adjusted toward a more fluid "genie" motion, but the resulting UI still looked wrong to the user because the arrow shape appeared inverted and the bubble border inherited an unwanted confidence-tinted outline. The takeaway is that animation polish should not ship before core visual correctness is nailed down, especially for overlays where arrow geometry and neutral chrome are immediately noticeable.

### Suggested Action
For future overlay and tooltip polish in sbobino, first verify arrow orientation, border treatment, and perceived visual hierarchy from a screenshot or live render, then iterate on motion styling.

### Metadata
- Source: user_feedback
- Related Files: sbobino_desktop/apps/desktop/src/styles.css, sbobino_desktop/apps/desktop/src/components/ConfidenceTranscript.tsx
- Tags: tooltip, overlay, animation, visual-regression

---
