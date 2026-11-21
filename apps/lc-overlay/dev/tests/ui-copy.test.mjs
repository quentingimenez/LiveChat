// Density of the overlay's own UI copy: the interface shows the settings and
// lists themselves, so no descriptive sentence introduces a section, a list or
// a device card. Hints stay only where they carry a fact the controls do not
// show (a multiplier, a cross-reference, a consequence, a constraint).
//
// The assertions read the view sources and the stylesheets on purpose: these
// views need a DOM and cannot be imported by Node. Absence assertions are the
// regression guard — reintroducing an intro sentence must fail here.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const admin = read('views/admin.js');
const settings = read('views/settings.js');
const guildSettings = read('views/guild-settings.js');
const profile = read('views/profile.js');
const adminCss = read('styles/admin.css');
const settingsCss = read('styles/settings.css');
const dialogCss = read('styles/dialog.css');

// A paragraph that only repeats what the headings and controls already say.
const removed = [
  // Administration lists.
  'Les membres de la guilde, avec leur rôle et leur dernière activité.',
  'Les comptes bannis de la guilde.',
  'Les overlays appairés à cette guilde, avec leur version et leur dernière activité.',
  'Les invitations et les codes d’appairage de la guilde.',
  'Les actions d’administration récentes de la guilde',
  // Profile devices.
  'Les appareils connectés à ton profil.',
  'Associe un autre PC, ton navigateur ou un raccourci iOS.',
  'Rattache un autre PC à ton profil.',
  'Envoie des médias depuis ton navigateur.',
  'Partage une photo, une vidéo ou un lien depuis l’app Raccourcis.',
  // Settings sections and their restating hints.
  'Réglages de cet appareil, appliqués à tous tes overlays.',
  'Guilde et cet appareil.',
  'Où et comment les médias s’affichent sur ton stream.',
  'Où et comment les médias de cette guilde s’affichent sur ton stream.',
  'Le son de tous tes overlays, guilde par guilde.',
  'Le niveau de cette guilde, appliqué par-dessus le volume général.',
  'Des touches pour agir sur l’overlay sans quitter ton jeu.',
  'Ton pseudo et ton avatar par défaut, dans chaque guilde que tu rejoins.',
  'LiveChat s’ouvre avec Windows, pour que tes overlays soient prêts.',
  'Le thème des fenêtres LiveChat et de la board.',
  'La version installée et ses mises à jour.',
  'Les médias partagés par la guilde, hébergés sur le serveur.',
  'Vérifie que les médias s’affichent et que le son sort.',
  'Sans avoir à lancer LiveChat toi-même après chaque redémarrage.',
  'Les médias restent affichés, sans leur son.',
  'Plus aucun média de cette guilde ne s’affiche sur cet appareil.',
  'Revient à la zone et à l’alignement des Paramètres de l’appareil.',
  '« Comme sur l’appareil » suit les Paramètres.',
  'Le nom affiché dans la liste des appareils de la guilde',
];

test('no removed explanatory sentence comes back', () => {
  const all = admin + settings + guildSettings + profile;
  for (const sentence of removed) {
    assert.ok(!all.includes(sentence), `the removed sentence is back: « ${sentence} »`);
  }
});

test('administration lists are introduced only by their tab and columns', () => {
  assert.doesNotMatch(admin, /\bintro\s*:/, 'renderList no longer takes an intro');
  assert.ok(!admin.includes('list-intro'), 'no list intro paragraph is rendered');
  assert.ok(!adminCss.includes('.list-intro'), 'the list intro rule is gone');
  // The structure stays: rows, column labels, explicit empty states.
  assert.match(admin, /labels: \['', 'Membre', 'Vu', 'Rôle', 'Action'\]/);
  assert.match(admin, /empty: 'Aucun membre dans cette guilde pour le moment\.'/);
});

test('settings sections have no description line', () => {
  assert.ok(!settings.includes('sectionDesc'), 'the section description helper is gone');
  assert.ok(!guildSettings.includes('sectionDesc'), 'guild settings use no section description');
  assert.doesNotMatch(settings + guildSettings, /settings-intro|settings-desc/);
  assert.ok(!settingsCss.includes('.settings-intro'), 'the page intro rule is gone');
  // The class still marks the server-side status lines (Discord link, commands).
  const statusLines = admin.match(/class: 'settings-desc'/g) || [];
  assert.equal(statusLines.length, 2, 'only the two Discord status lines keep the class');
  assert.match(admin, /'aria-describedby': 'set-default-hint'/);
  assert.match(admin, /Durée d’affichage d’une image ou d’un texte sans durée propre\./);
});

test('the autosave note is stated once, under the page title', () => {
  // One line per page replaces the tag each section used to repeat.
  const note = 'Paramètres sauvegardés automatiquement';
  assert.equal(settings.split(note).length - 1, 1, 'the note text is written once');
  // The call site is not the helper's declaration, which has no trailing comma.
  assert.equal((settings.match(/autosaveNote\(\),/g) || []).length, 1, 'the device settings state it once');
  assert.equal((guildSettings.match(/autosaveNote\(\),/g) || []).length, 1, 'the guild settings state it once');
  assert.ok(!(settings + guildSettings).includes('Appliqué immédiatement'), 'the repeated tag is gone');
  // Both pages keep it in their head, right after the title.
  assert.match(settings, /h\('h2', null, 'Paramètres'\),\s*autosaveNote\(\),/);
  assert.match(guildSettings, /h\('h2', null, 'Réglages de la guilde'\),\s*autosaveNote\(\),/);
});

test('profile devices have no card description', () => {
  assert.ok(!profile.includes('profile-device-desc'), 'no device description paragraph is rendered');
  assert.ok(!dialogCss.includes('.profile-device-desc'), 'the device description rule is gone');
  assert.ok(!dialogCss.includes('.profile-add-intro'), 'the add-device intro rule is gone');
  assert.ok(!dialogCss.includes('.profile-personal-intro'), 'the personal-devices intro rule is gone');
  // The tab names the list; count and refresh remain in its head.
  assert.match(profile, /'Mes appareils'/);
  assert.match(profile, /class: 'profile-personal-actions'/);
});

test('hints that carry a fact the controls do not show stay', () => {
  const kept = [
    'Multiplié par le volume général des Paramètres.',
    'Multiplié par le volume général de l’appareil.',
    'Ferme les connexions : plus rien n’est reçu ni affiché.',
    'Chaque guilde garde son propre profil, modifiable dans « Mon profil ».',
    '« Système » suit le thème clair/sombre de Windows.',
    'Affiche un numéro sur chaque écran pendant quelques secondes.',
    'Déplace la zone et tire ses coins sur l’écran de l’overlay',
    'Sans quota propre, la limite du serveur s’applique',
    'Vide = quota du serveur.',
    'Supprime tous les médias du board et les fichiers orphelins.',
  ];
  const all = admin + settings + guildSettings + profile;
  for (const text of kept) {
    assert.ok(all.includes(text), `the useful hint is gone: « ${text} »`);
  }
});
