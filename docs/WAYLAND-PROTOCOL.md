# Doc 2 — Wayland Protocol Design: `slug_semantic_v1`

**Milestone:** M0, M1  
**Status:** Final draft

-----

## 1. Newton Architecture Summary

GNOME’s **Newton** project (initiated 2024, led by Lukáš Tyrychtr and Emmanuele Bassi) is the reference redesign of the Linux accessibility stack for Wayland. Its key architectural decisions:

**Push-based, frame-synchronised delivery.** Newton hooks into the Mutter compositor’s frame commit path. After each frame is committed to the GPU, a accessibility tree snapshot/diff is synchronously computed and pushed to a session daemon over a Unix domain socket. This eliminates the polling model of AT-SPI2 (where screen readers call `GetChildren`, `GetName`, etc. on every object, causing N×M D-Bus round trips per frame).

**Wayland protocol extension, not X11 atoms.** Newton defines a `newton_accessibility_v1` Wayland protocol. Clients (toolkits) implement the `newton_accessible` interface, exposing role, state, name, and children. The compositor aggregates these into a surface-level tree.

**D-Bus for AT clients.** While the compositor-to-daemon path uses the Wayland socket (for frame synchronisation), the daemon-to-AT-client path uses D-Bus. This preserves the existing AT-SPI2 client interface, meaning screen readers (Orca) and other AT clients require no changes. It also provides the sandboxing boundary: AT clients do not have Wayland socket access.

**Toolkit-side implementation.** GTK 4 and Qt 6 are the primary toolkit targets. Each widget’s `GtkAccessible` implementation provides metadata; the toolkit’s Wayland backend serialises it into Newton protocol messages.

**Design decisions Newton explicitly made:**

1. No screenshot path — Newton is text/structure only.
1. Refs are compositor-assigned integers (object IDs in the Wayland protocol sense).
1. The initial snapshot is requested via a `get_tree` request; subsequent changes are pushed via events.
1. AT-SPI2 bridge in the daemon provides backward compatibility.

-----

## 2. Where Slug Diverges from Newton

|Dimension                |Newton                                    |Slug                                                                    |
|-------------------------|------------------------------------------|------------------------------------------------------------------------|
|Ref scheme               |Wayland object IDs (integers, per-session)|ULIDs (globally unique, stable across sessions for the embargo window)  |
|Primary consumer         |Human AT client (screen reader)           |LLM agent                                                               |
|AT-SPI2 bridge           |Mandatory (Orca compat)                   |Optional module — default OFF in headless agent mode                    |
|Initial snapshot delivery|On-demand `get_tree` request              |Mandatory push on first connection (agent cannot miss the initial state)|
|Node schema richness     |Role + state + name + bounds              |+ value_min/max/step, options[], actions[], validation, extensions      |
|Frame-sync hook          |Mutter-specific                           |wlroots-first, Mutter as secondary target                               |
|D-Bus schema             |AT-SPI2 compatible                        |New org.slug.Semantic interface (AT-SPI2 bridge is a separate adapter)  |

Slug’s compositor target is **wlroots** (Sway, river, Hyprland ecosystem) rather than Mutter, because wlroots exposes stable, documented hooks (`wlr_renderer`, `wlr_scene`, `wlr_output` commit callbacks) that Newton’s Mutter implementation relies on internal APIs for.

-----

## 3. `slug_semantic_v1` Protocol XML

```xml
<?xml version="1.0" encoding="UTF-8"?>
<protocol name="slug_semantic_v1">
  <copyright>
    Copyright 2026 Slug OS Contributors
    SPDX-License-Identifier: MIT
  </copyright>

  <description summary="Slug OS semantic UI tree protocol">
    This protocol allows a compositor to push a semantic accessibility tree
    to a session daemon. The tree represents the UI state of all visible
    surfaces in terms of roles, states, names, and geometry.

    The compositor implements slug_semantic_manager_v1.
    The session daemon binds slug_semantic_manager_v1 and creates
    slug_semantic_listener_v1 objects to receive tree events.

    Wire encoding: all node data is encoded as a compact binary blob
    (see slug_semantic_frame message) to minimise IPC overhead on
    high-frequency frame paths. The blob format is described in
    the Slug OS wire encoding specification (separate document).
  </description>

  <!-- ============================================================
       Manager — bound by the session daemon at startup
       ============================================================ -->
  <interface name="slug_semantic_manager_v1" version="1">
    <description summary="Manages semantic tree listeners">
      Singleton global. The session daemon calls create_listener once
      to establish a subscription. Multiple listeners are supported
      for multi-agent scenarios.
    </description>

    <request name="destroy" type="destructor">
      <description summary="Destroy the manager binding"/>
    </request>

    <request name="create_listener">
      <description summary="Create a new semantic tree listener">
        The compositor will begin pushing frame events to the returned
        listener object immediately after the next committed frame.
        A full snapshot is sent before the first delta frame.
      </description>
      <arg name="id" type="new_id" interface="slug_semantic_listener_v1"/>
      <arg name="capability_token" type="string"
           summary="Opaque token validated against the capability ledger"/>
    </request>

    <event name="capability_rejected">
      <description summary="Sent when capability_token is invalid or expired">
        The listener object is destroyed after this event.
      </description>
      <arg name="listener" type="object" interface="slug_semantic_listener_v1"/>
      <arg name="reason" type="string"/>
    </event>
  </interface>

  <!-- ============================================================
       Listener — receives tree events
       ============================================================ -->
  <interface name="slug_semantic_listener_v1" version="1">
    <description summary="Receives semantic tree events from the compositor">
      Events on this interface are delivered in frame order.
      The session daemon MUST process events in the order received.
    </description>

    <request name="destroy" type="destructor">
      <description summary="Unsubscribe from the semantic tree"/>
    </request>

    <request name="request_snapshot">
      <description summary="Request a full tree snapshot">
        The compositor will send a slug_snapshot event followed by
        resumption of normal delta events.
        Use after connection loss or tree corruption.
      </description>
    </request>

    <!-- Snapshot — sent on first connection and on request_snapshot -->
    <event name="slug_snapshot">
      <description summary="Full tree snapshot">
        Sent before the first delta. The session daemon MUST discard
        any prior tree state on receipt.
        payload_blob: length-prefixed binary blob containing an array
        of SlugNode records (see wire encoding spec).
      </description>
      <arg name="snapshot_id" type="uint"
           summary="Monotonically increasing snapshot counter"/>
      <arg name="timestamp_us" type="uint"
           summary="CLOCK_MONOTONIC timestamp in microseconds (low 32 bits)"/>
      <arg name="timestamp_us_hi" type="uint"
           summary="High 32 bits of timestamp_us"/>
      <arg name="node_count" type="uint"
           summary="Number of SlugNode records in payload_blob"/>
      <arg name="payload_blob" type="array"
           summary="Packed SlugNode array"/>
    </event>

    <!-- Delta frame — sent after each Wayland frame commit with changes -->
    <event name="slug_delta">
      <description summary="Incremental tree update">
        Sent for each frame that produced accessibility tree changes.
        Frames with no tree changes produce no event (compositor filters).

        payload_blob encodes a SlugDelta record:
          - created[]:   new SlugNode records
          - updated[]:   SlugNodePatch records (only changed fields)
          - destroyed[]: array of ref strings (ULIDs)
          - reordered[]: SlugReorder records
          - focus_ref:   ULID of newly focused node, or empty string
      </description>
      <arg name="frame_id" type="uint"
           summary="Frame sequence number, per surface_id"/>
      <arg name="surface_id" type="uint"
           summary="wl_surface.id of the surface that committed"/>
      <arg name="timestamp_us" type="uint"/>
      <arg name="timestamp_us_hi" type="uint"/>
      <arg name="payload_blob" type="array"/>
    </event>

    <!-- Window lifecycle -->
    <event name="window_opened">
      <description summary="A new top-level window appeared"/>
      <arg name="window_id" type="string"/>
      <arg name="surface_id" type="uint"/>
      <arg name="app_id" type="string"/>
      <arg name="title" type="string"/>
    </event>

    <event name="window_closed">
      <description summary="A top-level window was closed"/>
      <arg name="window_id" type="string"/>
    </event>

    <!-- Focus tracking shortcut (also in slug_delta, but duplicated
         here for clients that only need focus) -->
    <event name="focus_changed">
      <description summary="Keyboard focus moved to a different node"/>
      <arg name="ref" type="string" summary="ULID of newly focused node"/>
      <arg name="surface_id" type="uint"/>
    </event>

    <!-- Suspension — compositor cannot guarantee tree accuracy -->
    <event name="semantic_suspended">
      <description summary="Compositor is suspending semantic emission">
        Sent when the screen is locked, the session is switching,
        or the compositor is about to crash-recover.
        The session daemon MUST NOT send actions to the agent until
        semantic_resumed is received.
      </description>
      <arg name="reason" type="string"/>
    </event>

    <event name="semantic_resumed">
      <description summary="Semantic emission has resumed">
        A full snapshot follows this event.
      </description>
    </event>
  </interface>

  <!-- ============================================================
       Action channel — agent → compositor (bidirectional via daemon)
       Actions are NOT sent over this Wayland interface.
       They are sent via D-Bus to the session daemon, which
       forwards them to the compositor via a separate internal channel.
       This interface documents the compositor side of action dispatch.
       ============================================================ -->
  <interface name="slug_action_dispatcher_v1" version="1">
    <description summary="Internal: compositor side of action dispatch">
      This interface is bound only by the session daemon, never by
      the agent directly. The agent always goes through D-Bus.
    </description>

    <request name="destroy" type="destructor"/>

    <!-- The daemon calls this request to ask the compositor to
         perform an action on a node. -->
    <request name="dispatch_action">
      <description summary="Perform an action on a node">
        The compositor performs the action and sends action_result.
        If the ref is stale (node destroyed), action_result.success=0.
      </description>
      <arg name="action_token" type="string"
           summary="Opaque token for result correlation"/>
      <arg name="ref" type="string" summary="Target node ULID"/>
      <arg name="action_id" type="string"
           summary="e.g. activate, set_value, expand, scroll_into_view"/>
      <arg name="payload" type="string"
           summary="JSON-encoded action parameters"/>
    </request>

    <event name="action_result">
      <description summary="Result of a dispatch_action request"/>
      <arg name="action_token" type="string"/>
      <arg name="success" type="uint" summary="1=success, 0=failure"/>
      <arg name="error_message" type="string" summary="Empty on success"/>
    </event>
  </interface>

</protocol>
```

-----

## 4. Protocol Rationale

### 4.1 Binary blob payload vs typed Wayland args

Newton uses typed Wayland arguments (one arg per field). Slug uses a binary blob because:

- A single SlugNode has ~20 fields; a frame can create/update hundreds of nodes
- Typed args require one Wayland message per field change; blob packs an entire delta in one message
- The blob format (MessagePack) is self-describing enough to be debuggable with standard tools
- wl_array is the correct Wayland type for variable-length binary data

**Decision:** MessagePack over JSON for the blob because MessagePack is ~30% smaller and faster to parse, and the session daemon is on the hot path of every frame commit.

### 4.2 Why D-Bus for actions (not a second Wayland interface)

Actions (click, type text, set value) are sent from the agent → session daemon → compositor. Using D-Bus for this path:

- Provides a natural audit log (D-Bus monitor)
- Allows the session daemon to interpose a capability check before any action reaches the compositor
- Allows the action to be rate-limited, queued, or rejected without compositor changes
- Preserves the Wayland socket for the high-frequency read path only

The compositor’s `slug_action_dispatcher_v1` interface is a private interface between the compositor and the daemon — not exposed to the agent at all.

### 4.3 Frame synchronisation

The semantic delta is computed **synchronously in the compositor’s frame commit callback**, before the GPU buffer swap. This ensures the delta describes the frame that was actually displayed, not an earlier or later state. The alternative (async, post-frame) would introduce a 1-frame lag that would cause the agent to act on stale state.

### 4.4 Capability token scheme

The `capability_token` passed to `create_listener` is a 256-bit random token issued by the OS security daemon at session start. The compositor validates it against the capability ledger (see Doc 6). This prevents rogue processes from binding `slug_semantic_manager_v1` and reading UI state without authorisation.

### 4.5 wlroots vs Mutter target

Slug targets wlroots first because:

- wlroots is compositor-agnostic; a wlroots plugin works on Sway, Hyprland, river, and any other wlroots compositor
- Mutter (GNOME Shell) has Newton; Slug does not compete there in the short term
- The agent-first OS use case maps better to tiling compositors (Sway) than to GNOME Shell’s desktop metaphor
- A Mutter port is planned for M7 (not in scope for this dossier)

### 4.6 AT-SPI2 bridge

The session daemon includes an optional AT-SPI2 bridge module (`slug-atspi-bridge`) that translates the D-Bus `org.slug.Semantic` interface to `org.a11y.atspi` for compatibility with Orca and other screen readers. This module is disabled by default in agent-only mode but can be enabled for hybrid human+agent sessions.