# Rust Router

## Présentation

Ce projet implémente un protocole de routage dynamique en Rust, permettant à des routeurs de découvrir et de maintenir automatiquement les routes optimales dans un réseau local.  
Le protocole repose sur l’échange de messages "hello" entre voisins, la mise à jour automatique des routes, et la gestion des pannes.

---

## Prérequis

- **Rust** (édition stable, recommandé : 1.70+)
- **Cargo** (installé avec Rust)
- **Linux** (pour la gestion des routes système via `ip route`)
- Droits administrateur (`sudo`) pour modifier la table de routage

---

## Compilation

Clonez le dépôt et compilez le projet en release :

```sh
git clone https://github.com/Cessouille/rust-router.git
cd rust-router
cargo build --release
```

---

## Lancement

Lancez le routeur avec les droits administrateur :

```sh
sudo ./target/release/rust-router
```

---

## Utilisation

Une interface CLI s’affiche :

```
==============================
      Rust Router CLI
==============================
Dynamic routing: DISABLED
1. Enable/Disable dynamic routing
2. List last known neighbor routers
3. Exit
> Enter your choice:
```

- **1** : Active/désactive le routage dynamique (lancement/arrêt du protocole)
- **2** : Affiche la liste des voisins connus (découverts récemment)
- **3** : Quitte le programme proprement

---

## Fichier de logs

Les métriques de performance sont enregistrées dans le fichier `router_perf.log` (ou `report.log` si renommé) à la racine du projet.  
À chaque lancement, le fichier est réinitialisé.

Vous y trouverez :

- Le temps d’envoi des messages hello
- Le temps de réception des messages hello
- Le nombre de messages hello reçus
- La durée totale d’un cycle du protocole

---

## Points importants

- **Le programme doit être lancé avec les droits administrateur** pour pouvoir modifier la table de routage système.
- **Le routage dynamique doit être activé** (option 1) pour que le protocole fonctionne.
- **Les logs de performance** sont utiles pour analyser la stabilité et l’efficacité du protocole (voir le rapport pour l’analyse).

---

## Arrêt propre

Pour arrêter le routeur, choisissez l’option 3 dans le menu.  
Le thread de routage dynamique sera arrêté proprement et la table de routage ne sera plus modifiée.

---

## Dépannage

- Si vous ne voyez pas de voisins, vérifiez que les autres routeurs sont bien lancés et sur le même réseau.
- Si la table de routage ne change pas, vérifiez les droits administrateur et la configuration réseau.
- Consultez le fichier de logs pour diagnostiquer les performances ou les problèmes de convergence.

---

## Auteur

Projet réalisé dans le cadre d’un TP réseau, Rust 2025.
