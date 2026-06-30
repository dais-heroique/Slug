# Test de validation "vie réelle" — Amazon, achat d'un micro

Ce test est fait pour être collé tel quel dans Claude Code, **connecté à Slug en
MCP sur ton Mac** (`claude mcp add` ou équivalent déjà configuré). Il sert à
prouver, en conditions réelles, que Slug peut driver un navigateur (Safari ou
Chrome) jusqu'à un acte d'achat — sans aller jusqu'au paiement.

> ⚠️ **Le test s'arrête avant le paiement.** L'objectif est de prouver que Slug
> sait naviguer, lire une page de résultats, choisir un produit et l'ajouter au
> panier — pas de dépenser de l'argent. Si `slug_invoke` déclenche une demande
> d'approbation humaine (action jugée destructive — "buy", "order", "pay" sont
> des mots-clés qui la déclenchent), **réponds toujours "refuser"** dans le
> dashboard (`http://127.0.0.1:7333/dashboard`) ou laisse le timeout expirer.
>
> ⚠️ **Reste sur des `slug_snapshot` filtrés** (`filter`/`roles`/`limit`,
> comme dans chaque étape ci-dessous) — une page de résultats Amazon est dense
> et un snapshot non filtré (`interactive_only:false` sans `roles`/`filter`/
> `limit`) peut faire des centaines de Ko, ce qui dépasse ta propre limite de
> taille de résultat d'outil et te force dans un détour lent (dump fichier +
> grep). Slug tronque maintenant ce cas à ~20k caractères avec un message —
> mais autant ne pas en arriver là : filtre côté serveur dès le départ.

---

## Étape 0 — Statut

`slug_status {}` → vérifie `Accessibility bus: connected` et que le brain est
`ready`. Si la bus n'est pas connectée, arrête-toi et corrige les permissions
Accessibility avant de continuer (voir `SLUG-AGENT-GUIDE.md` §6).

## Étape 1 — Ouvrir Amazon

`slug_launch { "name": "Safari", "uri": "https://www.amazon.fr" }`
(remplace `.fr` par ton domaine Amazon local si besoin)
→ `isError:false`. Attends 2-3s que la page charge.

## Étape 2 — Chercher "microphone"

1. `slug_snapshot { "app": "Safari", "roles": ["entry", "field"], "filter": "search", "limit": 5 }`
   pour trouver la barre de recherche (sinon `slug_snapshot { "app": "Safari", "interactive_only": true, "limit": 30 }`
   pour voir toute la page).
2. Utilise `slug_sequence` pour rester atomique (évite le vol de focus) :
   ```json
   {
     "steps": [
       { "activate": "Safari" },
       { "wait_ms": 300 },
       { "ref": "<ref de la barre de recherche>", "action": "set_text", "args": "microphone" },
       { "key": "return" }
     ]
   }
   ```
   Si tu n'as pas de `ref` exploitable (page pas encore enrichie), utilise
   `slug_invoke { ref, action:"set_text", args:"microphone" }` puis
   `slug_key { keys:"return", activate:"Safari" }`.
3. Attends 2s, puis `slug_snapshot { "app": "Safari", "filter": "microphone", "limit": 20 }`
   pour vérifier que la page de résultats a chargé.

## Étape 3 — Choisir un produit

Deux stratégies possibles, choisis-en une et dis laquelle dans ton rapport :

- **A. Premier résultat** : prends le premier lien produit de la grille de
  résultats (généralement le premier `role:"link"` dont le texte contient un
  nom de marque/produit, pas une bannière publicitaire "Sponsorisé" — ignore
  les liens marqués "Sponsored"/"Sponsorisé" si tu peux les distinguer).
- **B. Meilleur rapport qualité/prix** : `slug_snapshot { "app": "Safari", "filter": "microphone", "limit": 30, "coords": true }`,
  lis les prix et notes (étoiles) affichés dans le texte des nœuds, choisis le
  produit avec le meilleur ratio note/prix parmi les 5-10 premiers résultats
  (évite les extrêmes : un produit à 2 notes ou à un prix anormalement bas).

`slug_invoke { "ref": "<ref du lien produit>", "action": "click", "reasoning": "ouvrir la fiche produit" }`
→ `isError:false`. Attends 2s.

## Étape 4 — Vérifier la fiche produit

`slug_snapshot { "app": "Safari", "filter": "panier", "limit": 10 }` (ou
`"Add to Cart"` / `"Ajouter au panier"` selon la langue) pour localiser le
bouton d'ajout au panier. Confirme avec un snapshot complet si besoin que tu es
bien sur une fiche produit (titre, prix, bouton d'achat visibles).

## Étape 5 — Ajouter au panier (S'ARRÊTE ICI)

`slug_invoke { "ref": "<ref du bouton panier>", "action": "click", "reasoning": "ajouter le micro au panier" }`

Deux issues possibles, les deux sont un succès du test :

- **isError:false sans approbation demandée** → le mot-clé "panier/cart" n'a
  pas déclenché le filtre destructif (c'est `is_destructive` côté
  `slug-core` ; seuls "buy"/"order"/"pay"/"delete"/"send" etc. le
  déclenchent). Le micro est dans le panier. **Ne va pas plus loin : ne clique
  pas sur "commander" / "passer la commande" / "payer".**
- **Une approbation humaine est demandée** (dashboard) → c'est le garde-fou de
  Slug qui fonctionne comme prévu. Refuse-la (`approved:false`) ou laisse le
  timeout expirer. Note ce comportement dans ton rapport : c'est une preuve
  que la protection contre les actions destructives marche, pas un bug.

## Étape 6 — Rapport final

Imprime un résumé court :

```
✅/❌ Recherche "microphone" sur Amazon
✅/❌ Stratégie utilisée : A (premier résultat) / B (meilleur rapport qualité/prix)
✅/❌ Produit trouvé : <nom>, <prix>
✅/❌ Ajout au panier : <isError, ou "approbation demandée et refusée — comportement attendu">
🛑 Arrêt confirmé avant paiement
```

Ne clique JAMAIS sur un bouton de paiement, de confirmation de commande, ou
n'entre aucune information de carte bancaire — ce n'est pas l'objet du test.
