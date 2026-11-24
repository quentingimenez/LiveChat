-- Contenu déjà importé par une guilde : un upload dont le SHA-256 est connu ici
-- peut être ajouté au board sans renvoyer les octets. La portée par guilde est
-- volontaire : le cache média global ne doit pas devenir un oracle qui laisse
-- une guilde réclamer le fichier privé d'une autre.
CREATE TABLE guild_upload_cache (
    guild_id TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (guild_id, source_hash)
);
