@0xb7d9f3a2c1e84d56;

# Canonical Cap'n Proto contract for the slug-ui bus.
#
# At this milestone the runtime ships these same message shapes as
# length-prefixed JSON frames over a Unix socket (see src/protocol.rs), so the
# Slug stack can consume them with no schema compiler. The encoder is meant to be
# swapped to Cap'n Proto without changing callers — this file is the source of
# truth for that wire format.

# A semantic node, derived automatically from a widget (never authored directly).
struct BusNode {
  ref      @0 :Text;     # agent-facing stable ref (derived ULID-shaped string)
  role     @1 :Text;     # SlugRole (snake_case)
  name     @2 :Text;
  value    @3 :Text;
  states   @4 :List(Text);
  actions  @5 :List(Text);   # click, set_text, set_value, toggle, increment, ...
  bounds   @6 :Bounds;
  children @7 :List(Text);   # child refs
}

struct Bounds {
  x @0 :Float64;
  y @1 :Float64;
  w @2 :Float64;
  h @3 :Float64;
}

# A high-level imperative tool (WebMCP navigator.modelContext-style).
struct ToolSpec {
  name         @0 :Text;
  description  @1 :Text;
  paramsSchema @2 :Text;   # JSON Schema, encoded as text
}

# A full semantic snapshot of an application.
struct BusSnapshot {
  app   @0 :Text;
  root  @1 :Text;          # root node ref
  nodes @2 :List(BusNode);
  tools @3 :List(ToolSpec);
}

# Agent -> app.
struct ClientMsg {
  union {
    snapshot @0 :Void;
    invoke   @1 :Invoke;
    callTool @2 :CallTool;
  }

  struct Invoke {
    ref    @0 :Text;
    action @1 :Text;
    args   @2 :Text;       # optional scalar argument
  }
  struct CallTool {
    name @0 :Text;
    args @1 :Text;         # JSON, encoded as text
  }
}

# App -> agent.
struct ServerMsg {
  union {
    snapshot     @0 :BusSnapshot;
    invokeResult @1 :InvokeResult;
    toolResult   @2 :ToolResult;
  }

  struct InvokeResult {
    ok    @0 :Bool;
    error @1 :Text;
  }
  struct ToolResult {
    ok    @0 :Bool;
    value @1 :Text;        # JSON, encoded as text
    error @2 :Text;
  }
}
