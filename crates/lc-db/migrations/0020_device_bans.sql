-- Migration 0020 — bannissement d'appareils.
--
-- Un bannissement ne s'arrête pas à l'identité Discord : un membre sans compte
-- lié revient avec une nouvelle invitation. L'overlay envoie un identifiant
-- d'appareil stable (MachineGuid Windows), haché côté serveur avant tout
-- stockage (`lc-device-v1|<guild_id>|<brut>`) ; le bannissement retient les
-- empreintes des appareils actifs du membre pour refuser leur retour.
--
-- L'empreinte n'est pas une authentification : elle est falsifiable, une
-- réinstallation du système en produit une autre, et les clients antérieurs au
-- champ `deviceId` n'en envoient pas. C'est une défense en profondeur.

ALTER TABLE overlay_clients ADD COLUMN device_hash TEXT;

-- `UNIQUE (guild_id, device_hash)` rattache un appareil au premier
-- bannissement de la guilde qui le vise. Deux membres partageant un appareil
-- banni en même temps ne le font donc porter que par le premier ban : lever
-- celui-ci libère l'appareil, même si l'autre ban reste posé. C'est une limite
-- assumée de la défense en profondeur (pas une authentification).
CREATE TABLE guild_ban_devices (
    ban_id      TEXT    NOT NULL REFERENCES guild_bans (id) ON DELETE CASCADE,
    guild_id    TEXT    NOT NULL,
    device_hash TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (ban_id, device_hash),
    UNIQUE (guild_id, device_hash)
) STRICT;

CREATE INDEX idx_guild_ban_devices_guild ON guild_ban_devices (guild_id);
