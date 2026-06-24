# Slug Validation Test — prompt pour Claude Code connecté à Slug via MCP

Tu es Claude Code connecté au serveur Slug. Exécute chaque test dans l'ordre.
Pour chaque test : appelle l'outil, lis le résultat, imprime une ligne :
`✅ PASS — <nom>` ou `❌ FAIL — <nom> : <raison>` ou `⏭ SKIP — <nom> : <raison>`.
Ne t'arrête pas sauf si le test lui-même l'exige. À la fin, imprime le tableau récap.

---

## PHASE 0 — Santé du serveur

**V0-1 · slug_help**
`slug_help {}` → `isError:false`, texte contient `slug_snapshot` et `slug_invoke`.

**V0-2 · slug_list_apps**
`slug_list_apps {}` → `isError:false`, au moins une app listée (Finder est toujours là).

**V0-3 · tools/list — 11 outils présents**
Vérifie que ces noms sont tous dans la liste :
`slug_snapshot slug_invoke slug_launch slug_click slug_scroll slug_key
slug_activate slug_sequence slug_wait_for slug_list_apps slug_help`

---

## PHASE 1 — Snapshot (ciblé par app, pas par focus)

> Utilise `app:"Finder"` pour tous les snapshots de cette phase — focus indépendant.

**V1-1 · snapshot app ciblé**
`slug_snapshot { "app": "Finder" }` → `isError:false`, YAML avec au moins 1 nœud.

**V1-2 · snapshot desktop**
`slug_snapshot { "scope": "desktop" }` → `isError:false`, plusieurs apps visibles.

**V1-3 · filtre roles:"clickable"**
`slug_snapshot { "app": "Finder", "roles": ["clickable"], "limit": 10 }`
→ `isError:false`, aucune ligne avec rôle `static_text` ou `group`.

**V1-4 · filtre texte + limit**
`slug_snapshot { "app": "Finder", "filter": "File", "limit": 3 }`
→ au plus 3 nœuds, chaque nom contient "File" (insensible à la casse).

**V1-5 · interactive_only**
`slug_snapshot { "app": "Finder", "interactive_only": true, "limit": 20 }`
→ aucun rôle `group`, `split_group`, `scroll_area`.

**V1-6 · coords:true imprime @x,y**
`slug_snapshot { "app": "Finder", "roles": ["button"], "limit": 3, "coords": true }`
→ au moins une ligne contient `@` suivi de chiffres.

**V1-7 · app inconnue → isError propre**
`slug_snapshot { "app": "ZZZNoSuchApp9999" }` → `isError:true`, pas de `error` JSON-RPC.

**V1-8 · paramètres schema présents**
Dans tools/list, `slug_snapshot.inputSchema.properties` contient :
`filter`, `roles`, `interactive_only`, `limit`, `app`, `coords`.

---

## PHASE 2 — slug_launch

**V2-1 · launch sans args → isError**
`slug_launch {}` → `isError:true`, texte contient "provide".

**V2-2 · launch TextEdit**
`slug_launch { "name": "TextEdit" }` → `isError:false`.

**V2-3 · launch avec URI fichier**
`slug_launch { "name": "TextEdit", "uri": "/etc/hosts" }` → `isError:false`.
(Ferme l'onglet après si tu veux, pas obligatoire.)

**V2-4 · launch Safari avec URL**
`slug_launch { "name": "Safari", "uri": "https://example.com" }` → `isError:false`.

---

## PHASE 3 — slug_invoke (actions)

> TextEdit doit être ouvert (V2-2 l'a lancé).

**V3-1 · set_text sur entry_multiline**
1. `slug_snapshot { "app": "TextEdit", "roles": ["entry_multiline", "entry", "field"], "limit": 5 }`
   Si vide : `slug_snapshot { "app": "TextEdit" }` pour voir l'arbre complet.
2. Prends le premier ref trouvé.
3. `slug_invoke { "ref": "<ref>", "action": "set_text", "args": "slug-test-ok", "reasoning": "validation" }`
→ `isError:false`, texte contient "ok" ou "dispatched".

**V3-1b · set_text marche même sur [disabled]**
Si le nœud en V3-1 montrait `[disabled]` dans l'arbre et que le set_text a quand même
retourné ok → note-le dans le résultat. C'est le comportement attendu, pas un bug.

**V3-2 · focus sur un bouton**
`slug_snapshot { "app": "Finder", "roles": ["button"], "limit": 1 }`
→ prends le ref, `slug_invoke { "ref": "<ref>", "action": "focus", "reasoning": "test focus" }`
→ `isError:false`.

**V3-3 · action sur app inconnue/ref inexistant → isError propre**
`slug_invoke { "ref": "z999", "action": "click", "reasoning": "test stale ref" }`
→ `isError:true`, pas de protocol error.

---

## PHASE 4 — slug_key (AVEC activate obligatoire)

> Règle absolue : toujours passer `activate` pour les chords.
> Sans `activate`, le chord atterrit dans la fenêtre Claude Code, pas dans l'app cible.

**V4-1 · mode text avec activate**
`slug_key { "keys": "bonjour slug", "mode": "text", "activate": "TextEdit", "reasoning": "type test" }`
→ `isError:false`, résultat dit "ok: sent text … to TextEdit".

**V4-2 · chord cmd+a avec activate**
`slug_key { "keys": "cmd+a", "activate": "TextEdit", "reasoning": "select all in TextEdit" }`
→ `isError:false`, résultat dit "ok: sent chord … to TextEdit".
Ensuite : `slug_snapshot { "app": "TextEdit", "roles": ["entry_multiline", "entry"], "limit": 3 }`
pour vérifier que TextEdit a bien réagi (état `[selected]` ou texte).

**V4-3 · chord cmd+z avec activate**
`slug_key { "keys": "cmd+z", "activate": "TextEdit", "reasoning": "undo" }` → `isError:false`.

**V4-4 · keys vide → isError**
`slug_key {}` → `isError:true`.

**V4-5 · slug_key sans activate — vérifier le message d'avertissement**
`slug_key { "keys": "cmd+s", "reasoning": "test sans activate" }`
→ `isError:false` mais le texte de retour DOIT mentionner "frontmost" ou "activate"
   pour avertir que la destination est inconnue.
   Si le texte dit juste "ok: sent …" sans avertissement → `❌ FAIL` (régression).

---

## PHASE 5 — slug_sequence (atomique, sans focus theft)

**V5-1 · steps vides → isError avec "empty"**
`slug_sequence { "steps": [] }` → `isError:true`, texte contient "empty".

**V5-2 · wait_ms seul fonctionne sans bus**
`slug_sequence { "steps": [ { "wait_ms": 1 } ] }` → `isError:false`, "ran 1 steps".

**V5-3 · activate + text + key — séquence atomique complète**
`slug_sequence { "steps": [
  { "activate": "TextEdit" },
  { "wait_ms": 150 },
  { "text": "sequence-test" },
  { "key": "return" }
] }` → `isError:false`, "ran 4 steps".
Ensuite vérifie avec `slug_snapshot { "app": "TextEdit" }` que "sequence-test" est dans le document.

**V5-4 · description mentionne "atomic" et "focus"**
Dans tools/list, `slug_sequence.description` contient "atomic" ET "focus".

---

## PHASE 6 — slug_activate

**V6-1 · activer Finder**
`slug_activate { "app": "Finder" }` → `isError:false`.

**V6-2 · app inconnue → isError**
`slug_activate { "app": "ZZZNoSuchApp" }` → `isError:true`.

---

## PHASE 7 — slug_click et slug_scroll

**V7-1 · click valide (bouton Finder)**
`slug_snapshot { "app": "Finder", "roles": ["button"], "limit": 3, "coords": true }`
→ lis un `@x,y` d'un bouton non-destructif (toolbar).
`slug_click { "x": <x>, "y": <y> }` → `isError:false`.

**V7-2 · click sans y → isError**
`slug_click { "x": 100 }` → `isError:true`, texte contient `'y'` ou "required".

**V7-3 · scroll dans Finder**
`slug_snapshot { "app": "Finder", "roles": ["scroll_area"], "limit": 3, "coords": true }`
→ prends un `@x,y`, `slug_scroll { "x": <x>, "y": <y>, "dy": -3 }` → `isError:false`.

---

## PHASE 8 — slug_wait_for

**V8-1 · timeout court expire proprement**
`slug_wait_for { "event_type": "node_created", "timeout_ms": 300 }`
→ `isError:false`, texte contient "timeout" (ou un event si un coincide, c'est ok aussi).

---

## PHASE 9 — Limitations OS connues

**V9-1 · Spotify (si installé) — opaque ou generic**
`slug_launch { "name": "Spotify" }`, attends 2s, puis
`slug_snapshot { "app": "Spotify", "limit": 10 }`.
Si Spotify absent : SKIP.
Si présent : `isError:false` ET le résultat est soit vide / `generic` sans enfants
(opaque) soit a des nœuds (enrichi). Dans les deux cas, pas de protocol error.

**V9-2 · Spotify drivable via slug_key malgré opaque**
Si V9-1 a trouvé Spotify :
`slug_key { "keys": "space", "activate": "Spotify", "reasoning": "play/pause test" }`
→ `isError:false`. (L'event part même sans arbre AX.)

**V9-3 · Notes — écriture via sequence**
`slug_launch { "name": "Notes" }`.
`slug_sequence { "steps": [{"activate":"Notes"}, {"wait_ms":300}, {"text":"slug-ok"}] }`
→ `isError:false`, "ran 3 steps". (Le corps est WKWebView — write-only via AX, pas de lecture.)

---

## PHASE 10 — Contrats d'erreur JSON-RPC

**V10-1 · outil inconnu → code -32602**
`tools/call` avec `name: "slug_nope"` → `error.code` vaut `-32602`.

**V10-2 · méthode inconnue → code -32601**
Requête JSON-RPC `method: "does/not/exist"` → `error.code` vaut `-32601`.

---

## PHASE 11 — Multi-moniteur (si applicable)

**V11-1 · coordonnées négatives sur second écran**
`slug_snapshot { "scope": "desktop", "coords": true, "limit": 80 }`
→ Si double écran : au moins un nœud avec `@-` (X négatif). PASS avec note.
→ Si écran unique : SKIP (attendu, pas un bug).

---

## RÉSUMÉ FINAL

Imprime ce tableau :

| # | Test | Résultat |
|---|------|----------|
| V0-1 | slug_help | ✅/❌/⏭ |
| V0-2 | slug_list_apps | … |
| … | … | … |

Puis :
- **PASS** : X
- **FAIL** : X — liste chaque échec avec le texte exact reçu vs attendu
- **SKIP** : X

Si un test FAIL, donne le texte brut retourné par Slug pour que le développeur
puisse reproduire et corriger.
