# Slug — checklist de test sur Mac (scénario Canva)

But : valider chaque brique **isolément** (déterministe, sans IA) puis le **scénario complet** (avec l'IA). Note pour chaque étape : ✅ marche / ⚠️ marche à moitié / ❌ casse, + ce que tu observes.

Les commandes `curl` parlent au daemon MCP en direct → elles testent la **tech** sans dépendre du raisonnement de l'IA. C'est le meilleur moyen d'isoler un problème.

---

## Phase 0 — Mise à jour & permissions

```sh
cd ~/Slug
git pull origin claude/slug-m25-multiprovider
./slug-install/install.sh
```

Permissions (les deux, sur `~/.slug/bin/slug-mcp`) :
- **System Settings → Privacy & Security → Accessibility** → ajouter `~/.slug/bin/slug-mcp` (glisser depuis Finder, `⌘⇧G` → `~/.slug/bin`), interrupteur **ON**
- **System Settings → Privacy & Security → Input Monitoring** → ajouter le même binaire, **ON**

Puis redémarre le daemon :
```sh
launchctl kickstart -k gui/$(id -u)/org.slug.daemon
```

Petit helper pour la suite (appel d'un outil MCP en une ligne) :
```sh
slugcall () { curl -s -X POST http://127.0.0.1:7333/mcp \
  -H 'content-type: application/json' -H 'Origin: http://127.0.0.1:7333' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" ; echo ; }
```

- [ ] **0a.** `curl -s http://127.0.0.1:7333/healthz` → `ok`
- [ ] **0b.** `slugcall slug_list_apps '{}'` → liste d'apps (PAS « not connected » / « permission denied »). Si erreur permission → revois Phase 0.

---

## Phase 1 — Perception (lecture, le cœur)

- [ ] **1a.** Ouvre **TextEdit** à la main. Puis :
  `slugcall slug_snapshot '{"scope":"focused"}'`
  → tu dois voir un arbre YAML avec `window "…"`, des `button`, etc. et des `[ref=…]`.
- [ ] **1b.** Vérifie que les refs ont du sens (boutons nommés, champ texte, etc.).
- [ ] **1c.** Latence : relance 1a plusieurs fois — ça doit répondre **vite** (l'app focalisée seule est scannée).

---

## Phase 2 — Lancer une app (`slug_launch`)

- [ ] **2a.** `slugcall slug_launch '{"name":"Safari"}'` → Safari s'ouvre.
- [ ] **2b.** Deep link : `slugcall slug_launch '{"name":"Safari","uri":"https://www.canva.com"}'` → Safari ouvre Canva.
- [ ] **2c.** (Si tu as l'app) `slugcall slug_launch '{"name":"Spotify"}'` → Spotify s'ouvre.

> Si `name` ne marche pas pour une app, essaie le nom exact tel qu'affiché dans `/Applications`.

---

## Phase 3 — Cliquer DANS l'app via ref (le clic « propre »)

Avec TextEdit au premier plan :
- [ ] **3a.** `slugcall slug_snapshot '{"scope":"focused"}'` → repère un bouton, note son `ref` (ex. `b3`).
- [ ] **3b.** `slugcall slug_invoke '{"ref":"b3","action":"click","reasoning":"test clic"}'`
  → le bouton réagit (menu s'ouvre, etc.). C'est le **vrai clic dans l'app**.
- [ ] **3c.** Champ texte : repère un `entry [ref=iX]`, puis
  `slugcall slug_invoke '{"ref":"iX","action":"set_text","args":"bonjour slug"}'` → le texte apparaît.

---

## Phase 4 — Clavier synthétique (`slug_key`)

App au premier plan (TextEdit) :
- [ ] **4a.** Texte littéral : `slugcall slug_key '{"keys":"hello world","mode":"text"}'` → « hello world » se tape.
- [ ] **4b.** Raccourci : `slugcall slug_key '{"keys":"cmd+a"}'` → tout se sélectionne.
- [ ] **4c.** `slugcall slug_key '{"keys":"cmd+c"}'` puis `'{"keys":"cmd+v"}'` → copie/colle.
- [ ] **4d.** Navigation : `slugcall slug_key '{"keys":"down"}'`, `'{"keys":"return"}'`, `'{"keys":"escape"}'`.

> Si rien ne se passe : c'est presque toujours la permission **Input Monitoring** manquante. Re-vérifie Phase 0 + kickstart.

---

## Phase 5 — Clic souris aux coordonnées (`slug_click`)

- [ ] **5a.** `slugcall slug_click '{"x":400,"y":300,"reasoning":"test souris"}'` → le curseur clique à cet endroit de l'écran.
- [ ] **5b.** Dans un snapshot, cherche une zone opaque `generic … @x,y` ou `image … @x,y` ; clique son centre :
  `slugcall slug_click '{"x":<x>,"y":<y>}'`.

---

## Phase 6 — Scénario complet Canva (avec l'IA)

Deux façons : le **dashboard** (http://127.0.0.1:7333/dashboard, boîte de tâche) ou **Claude Code**. Donne une consigne claire, en précisant les libellés tels qu'affichés.

- [ ] **6a.** Tâche : `Ouvre Canva dans Safari, clique sur "Créer un design", puis sélectionne le modèle "Logo"`
- [ ] **6b.** Observe dans le journal (dashboard, colonne droite) : l'IA fait bien `slug_launch` → `slug_snapshot` → `slug_invoke click` → re-`slug_snapshot`.
- [ ] **6c.** Note où ça bloque, le cas échéant :
  - L'IA ne « voit » pas le bouton ? (→ le navigateur n'expose peut-être pas l'a11y web ; teste **Safari** plutôt que Chrome)
  - Le clic ne fait rien ? (→ note le `ref` et l'erreur exacte)
  - Le canvas d'édition est vide dans le snapshot ? (→ attendu : zone opaque ; vérifie qu'il y a un `@x,y`)

---

## Ce que tu me remontes

Pour chaque phase, copie-moi :
1. Le **statut** (✅/⚠️/❌) par étape.
2. Pour les ❌ : le **texte de retour** de la commande (le champ `text`) et ce que tu as vu à l'écran.
3. Pour Canva : colle le **journal d'actions** du dashboard (ou les appels d'outils côté Claude).
4. Les logs si besoin : `tail -n 50 ~/Library/Logs/slug/slug-mcp.err.log`

Avec ça je corrige précisément ce qui coince (mapping de touches, lecture des bounds, libellés, etc.).
```
