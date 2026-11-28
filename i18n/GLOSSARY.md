# Glossary — one word per concept, in every app

Agreed on 2026-09-24 across the Apple apps and Android, and binding for every
client and the catalog in `strings/`. When two apps disagree, a wrong
translation loses; otherwise the more common word wins, and this page is updated.
Czech always uses formal address (vykání).

English is **American English in every app** (decided 2026-11-16): Favorite,
Color, Catalog, Canceled, recognize. The English column is the word the UI
shows.

**Slovak, Hungarian, German, French, Spanish and Italian were written by
machine on 2026-11-22 and have not been checked by a native speaker.** Slovak
and Hungarian start from the box's UI text (`own-audio-box/ui/i18n.js`); the
other four follow the words Apple's own apps (Music, Books, Podcasts) show in
that language, from memory, not checked against a device. Treat these columns
as a proposal until the open item below is closed. English and Czech are
unchanged and stay binding.

Rows from "sign up" down were added on 2026-11-22 for the paid-work features
and the player; their Czech is the word the catalog in `strings/` (or the box)
already uses, not a new decision.

| English | Czech | Slovak | Hungarian | German | French | Spanish | Italian | Notes |
|---|---|---|---|---|---|---|---|---|
| audiobook | audiokniha | audiokniha | hangoskönyv | Hörbuch | livre audio | audiolibro | audiolibro | |
| podcast | podcast | podcast | podcast | Podcast | podcast | pódcast | podcast | es: RAE spelling *pódcast*; Apple es writes "podcast" — pick one when es is checked |
| music | hudba | hudba | zene | Musik | musique | música | musica | |
| show (podcast) | pořad | relácia | műsor | Sendung | émission | programa | programma | sk/hu from the box |
| episode | epizoda | epizóda | epizód | Folge | épisode | episodio | episodio | not "díl". de: Folge, not Episode (Apple Podcasts) |
| feed | kanál | kanál | csatorna | Feed | flux | feed | feed | tab "Feeds" = Kanály. de/es/it keep the English word, as their podcast apps do |
| Follow / Unfollow (podcasts) | odebírat / Zrušit odběr | odoberať / Zrušiť odber | Követés / Követés leállítása | Folgen / Nicht mehr folgen | Suivre / Ne plus suivre | Seguir / Dejar de seguir | Segui / Non seguire più | decided 2026-11-16: never "Subscribe" in English; not "sledovat" or "Odhlásit" in Czech. sk follows the box and the Czech decision, although Apple sk says "Sledovať". de: "Folgen" also means "episodes"; Apple lives with it, so do we |
| track / song | skladba | skladba | szám | Titel | morceau | canción | brano | hu "szám" from the box; de "Titel" as Apple Music de |
| album | album | album | album | Album | album | álbum | album | plural alba; sk albumy, hu albumok, de Alben, it invariable |
| artist | interpret | interpret | előadó | Künstler | artiste | artista | artista | |
| playlist | playlist | playlist | lejátszási lista | Playlist | playlist | playlist | playlist | hu: "lista" alone where the context is clear (the box does this); es/fr feminine (la playlist), de feminine (die Playlist) |
| collection | sbírka | zbierka | gyűjtemény | Sammlung | collection | colección | raccolta | |
| library | knihovna | knižnica | könyvtár | Mediathek | bibliothèque | biblioteca | libreria | de "Mediathek", it "Libreria" as Apple Music |
| cover | obálka | obálka | borító | Cover | pochette (music) / couverture (book) | portada | copertina | not "obal": Bez obálky, Stáhnout obálku, Nastavit obálku… fr is the one exception to one word: no French word fits both an album and a book |
| metadata | metadata | metadáta | metaadatok | Metadaten | métadonnées | metadatos | metadati | not "údaje" |
| identify / re-identify | Identifikovat / Znovu identifikovat | Identifikovať / Znova identifikovať | Azonosítás / Újbóli azonosítás | Identifizieren / Erneut identifizieren | Identifier / Identifier à nouveau | Identificar / Volver a identificar | Identifica / Identifica di nuovo | never "Určit". Music recognition against MusicBrainz, not Shazam-style listening |
| Up Next | Na řadě | Na rade | Következik | Als Nächstes | À suivre | A continuación | A seguire | the box labels its queue "Fronta / Front / Lejátszási sor"; sk "Front" is a bohemism; standard Slovak is "poradie" or "rad" |
| Offline Mode | Režim offline | Režim offline | Offline mód | Offline-Modus | Mode hors ligne | Modo sin conexión | Modalità offline | |
| downloads | Stažené | Stiahnuté | Letöltések | Downloads | Téléchargements | Descargas | Scaricati | |
| Skip back / forward | Přeskočit zpět / vpřed | Preskočiť späť / dopredu | Visszaugrás / Előreugrás | Zurückspringen / Vorspringen | Reculer / Avancer | Retroceder / Avanzar | Indietro / Avanti | plain "Back/Forward N seconds" = Zpět/Vpřed o N sekund; sk Späť/Dopredu o N sekúnd; hu N másodperccel vissza/előre; de N Sekunden zurück/vor; fr Reculer/Avancer de N secondes; es Retroceder/Avanzar N segundos; it Indietro/Avanti di N secondi |
| Lyrics | Text | Text | Dalszöveg | Songtext | Paroles | Letra | Testo | |
| Muted music | Ztlumená hudba | Stlmená hudba | Némított zene | Stummgeschaltete Musik | Musique coupée | Música silenciada | Musica silenziata | decided 2026-11-16 |
| Visualizer | Vizualizace | Vizualizácia | Vizualizáció | Visualizer | Visualiseur | Visualizador | Visualizzatore | |
| No limit | Bez omezení | Bez obmedzenia | Nincs korlát | Unbegrenzt | Illimité | Sin límite | Nessun limite | |
| Home (tab) | Domů | Domov | Kezdőlap | Home | Accueil | Inicio | Home | "on Home" = na kartě Domů. sk/hu from the box; de/it keep "Home" as Apple Music does |
| home screen (the device's) | plocha | plocha | Főképernyő | Home-Bildschirm | écran d'accueil | pantalla de inicio | schermata Home | "Přidat na plochu" — the words Czech iOS and Android show. The Apple apps still say "Domovská obrazovka"; move them to plocha when their catalogs are next edited |
| Continue listening | Pokračovat v poslechu | Pokračovať v počúvaní | Hallgatás folytatása | Weiterhören | Reprendre l'écoute | Seguir escuchando | Continua ad ascoltare | sk/hu from the box |
| Stats | Statistiky | Štatistiky | Statisztika | Statistik | Statistiques | Estadísticas | Statistiche | |
| storage | úložiště | úložisko | tárhely | Speicher | stockage | almacenamiento | spazio di archiviazione | it: "spazio" alone in tight spots |
| credit | kredit | kredit | egyenleg | Guthaben | crédit | saldo | credito | prepaid money. hu "egyenleg" (balance) and es "saldo" are what prepaid phone credit is called; "kredit"/"crédito" read as a loan |
| Storage & credit (the credit page) | Úložiště a kredit | Úložisko a kredit | Tárhely és egyenleg | Speicher und Guthaben | Stockage et crédit | Almacenamiento y saldo | Spazio e credito | decided 2026-11-16: never "Billing" / "Fakturace" (nor Abrechnung, Facturation, Facturación, Fatturazione, Számlázás, Fakturácia) |
| family | rodina | rodina | család | Familie | famille | familia | famiglia | |
| trash | koš | kôš | Kuka | Papierkorb | Corbeille | Papelera | Cestino | |
| settings | Nastavení | Nastavenia | Beállítások | Einstellungen | Réglages | Ajustes | Impostazioni | fr "Réglages", es "Ajustes" as Apple (Android says Paramètres / Configuración; use Apple's in every app) |
| sign in / sign out | Přihlásit se / Odhlásit se | Prihlásiť sa / Odhlásiť sa | Bejelentkezés / Kijelentkezés | Anmelden / Abmelden | Se connecter / Se déconnecter | Iniciar sesión / Cerrar sesión | Accedi / Esci | |
| upload | Nahrát | Nahrať | Feltöltés | Hochladen | Importer | Subir | Carica | fr: "Téléverser" is Québec usage; France says Importer |
| remove | Odebrat | Odobrať | Eltávolítás | Entfernen | Retirer | Quitar | Rimuovi | from a list or library; Smazat = delete for good. Same split in every language: Retirer/Supprimer, Quitar/Eliminar, Rimuovi/Elimina, Entfernen/Löschen |
| delete | Smazat | Zmazať | Törlés | Löschen | Supprimer | Eliminar | Elimina | |
| Cancel / Done | Zrušit / Hotovo | Zrušiť / Hotovo | Mégse / Kész | Abbrechen / Fertig | Annuler / OK | Cancelar / OK | Annulla / Fine | fr/es "OK" for Done, as Apple |
| Light / Dark / System (theme) | Světlý / Tmavý / Podle systému | Svetlý / Tmavý / Podľa systému | Világos / Sötét / Rendszer szerint | Hell / Dunkel / Wie System | Clair / Sombre / Système | Claro / Oscuro / Sistema | Chiaro / Scuro / Sistema | |
| duration | `1 h 20 min` | `1 h 20 min` | `1 ó 20 p` | `1 Std. 20 Min.` | `1 h 20 min` | `1 h 20 min` | `1 h 20 min` | same form in both languages. The other six take the OS's abbreviated duration formatter's output; the cells show what it gives |
| Just me (not shared) | Jen já | Len ja | Csak én | Nur ich | Moi uniquement | Solo yo | Solo io | decided 2026-11-16 for every app; never "Private" / "soukromé" for the section; nothing is locked |
| Make private / Share with the family | Nastavit jako soukromé / Sdílet s rodinou | Nastaviť ako súkromné / Zdieľať s rodinou | Priváttá tétel / Megosztás a családdal | Privat machen / Mit der Familie teilen | Rendre privé / Partager avec la famille | Hacer privado / Compartir con la familia | Rendi privato / Condividi con la famiglia | "Make private" is allowed for the action only |
| Who can see / hear this | Kdo to uvidí / Kdo to uslyší | Kto to uvidí / Kto to bude počuť | Ki láthatja? / Ki hallhatja? | Wer sieht das? / Wer hört das? | Qui peut le voir ? / Qui peut l'écouter ? | ¿Quién puede verlo? / ¿Quién puede escucharlo? | Chi può vederlo? / Chi può ascoltarlo? | |
| Follow a podcast (action) | Přidat podcast | Pridať podcast | Podcast hozzáadása | Podcast hinzufügen | Ajouter un podcast | Añadir podcast | Aggiungi podcast | the ongoing state is odebírat (and its column above) |
| mark played / unplayed | Označit jako přehrané / nepřehrané | Označiť ako vypočuté / nevypočuté | Megjelölés lejátszottként / lejátszatlanként | Als gespielt markieren / Als nicht gespielt markieren | Marquer comme lu / non lu | Marcar como reproducido / no reproducido | Segna come riprodotto / non riprodotto | sk/hu from the box; fr "lu" as Apple Podcasts fr |
| catalog (podcast directory) | katalog | katalóg | katalógus | Katalog | catalogue | catálogo | catalogo | |
| tags (on a book, the user's) | štítky | štítky | címkék | Tags | étiquettes | etiquetas | etichette | file metadata tags are značky; sk značky, hu fájlcímkék, de Datei-Tags, fr balises, es etiquetas del archivo, it tag del file |
| credits (people on a book) | tvůrci | tvorcovia | közreműködők | Mitwirkende | intervenants | participantes | partecipanti | narrator = vypravěč; sk rozprávač, hu felolvasó, de Sprecher/Sprecherin, fr narrateur/narratrice, es narrador/narradora, it narratore/narratrice. fr/es/it never "crédits/créditos/crediti": it collides with the money credit |
| narrate (text-to-speech a book) | namluvit / namlouvání | nahovoriť / nahovorenie | felolvastatás / felolvasás | vertonen / Vertonung | narrer / narration | narrar / narración | narrare / narrazione | never "načíst", which reads as "load". The Apple apps may still say načíst; move them when their catalogs are next edited. sk never "načítať" for the same reason. "Narrated by {n}" for a human reader: sk Číta {n}, hu Felolvasó: {n} (the box) |
| admin (role) / member | správce / člen | správca / člen | adminisztrátor / tag | Administrator / Mitglied | administrateur / membre | administrador / miembro | amministratore / membro | the server's admin is správce serveru (sk správca servera, hu szerveradminisztrátor, de Serveradministrator, fr administrateur du serveur, es administrador del servidor, it amministratore del server) |
| age brackets | Dospělý / Dospívající / Dítě | Dospelý / Dospievajúci / Dieťa | Felnőtt / Tinédzser / Gyermek | Erwachsener / Jugendlicher / Kind | Adulte / Ado / Enfant | Adulto / Adolescente / Niño | Adulto / Ragazzo / Bambino | |
| session (one listen) | poslech | počúvanie | hallgatás | Hörsitzung | écoute | escucha | ascolto | |
| streak | série | séria | sorozat | Serie | série | racha | serie | hu: "sorozat" is also a book series; prefer "N napja zsinórban" in sentences |
| year in review ("Wrapped") | … v kostce | … v kocke | … dióhéjban | … im Rückblick | … en bref | … en resumen | … in breve | "Váš rok v kostce"; sk "Váš rok v kocke", hu "Az Ön éve dióhéjban", de "Dein Jahr im Rückblick", fr "Votre année en bref", es "Tu año en resumen", it "Il tuo anno in breve" |
| widget | widget | widget | widget | Widget | widget | widget | widget | |
| filter (noun / verb) | filtr / filtrovat | filter / filtrovať | szűrő / szűrés | Filter / filtern | filtre / filtrer | filtro / filtrar | filtro / filtrare | |
| Couldn't … (errors) | … se nezdařilo / Nepodařilo se … | Nepodarilo sa … | Nem sikerült … | … konnte nicht … werden | Impossible de … | No se ha podido … | Impossibile … | one wording per message, reused. sk/hu from the box |
| sign up | Vytvořit účet | Vytvoriť účet | Fiók létrehozása | Konto erstellen | Créer un compte | Crear cuenta | Crea account | never "Register" / "Registrovat" in the UI |
| invite / invite code | pozvánka / kód pozvánky | pozvánka / kód pozvánky | meghívó / meghívókód | Einladung / Einladungscode | invitation / code d'invitation | invitación / código de invitación | invito / codice di invito | |
| server | server | server | szerver | Server | serveur | servidor | server | |
| Machine translation (badge) | Strojový překlad | Strojový preklad | Gépi fordítás | Maschinelle Übersetzung | Traduction automatique | Traducción automática | Traduzione automatica | `badge.machineTranslation`: describes the item, not a warning |
| AI voice (badge) | Hlas AI | Hlas AI | MI-hang | KI-Stimme | Voix IA | Voz IA | Voce IA | `badge.aiVoice`. The local abbreviation: hu MI, de KI, fr/es/it IA |
| text preparation | Příprava textu | Príprava textu | Szöveg előkészítése | Textaufbereitung | Préparation du texte | Preparación del texto | Preparazione del testo | quote line: abbreviations, numbers, dialogue made ready for the voice |
| price estimate (quote) | odhad / Celková cena | odhad / Celková cena | becslés / Teljes ár | Schätzung / Gesamtpreis | estimation / Prix total | estimación / Precio total | stima / Prezzo totale | the shown price is the most it can cost; never "offer" / "Nabídka" (Nabídka is the menu) |
| Add credit (top up) | Přidat kredit | Pridať kredit | Egyenleg feltöltése | Guthaben aufladen | Ajouter du crédit | Recargar saldo | Ricarica credito | |
| Favorites | Oblíbené | Obľúbené | Kedvencek | Favoriten | Favoris | Favoritos | Preferiti | add / remove: Přidat do oblíbených / Odebrat z oblíbených; sk/hu from the box |
| Shuffle | Náhodně | Náhodne | Keverés | Zufällig | Aléatoire | Aleatorio | Casuale | |
| Repeat | Opakovat | Opakovať | Ismétlés | Wiederholen | Répéter | Repetir | Ripeti | |
| smart playlist | chytrý playlist | chytrý playlist | okos lista | intelligente Playlist | playlist intelligente | playlist inteligente | playlist smart | from the box (cs/sk/hu); de/fr/es/it as Apple Music |
| chapter | kapitola | kapitola | fejezet | Kapitel | chapitre | capítulo | capitolo | |
| bookmark | záložka | záložka | könyvjelző | Lesezeichen | signet | marcador | segnalibro | |
| Sleep timer | Časovač vypnutí | Časovač vypnutia | Elalvási időzítő | Schlaftimer | Minuteur de veille | Temporizador de apagado | Timer di spegnimento | sk/hu from the box |
| Playback speed | Rychlost přehrávání | Rýchlosť prehrávania | Lejátszási sebesség | Wiedergabegeschwindigkeit | Vitesse de lecture | Velocidad de reproducción | Velocità di riproduzione | "Speed" alone in the player: Rychlost, Rýchlosť, Sebesség, Tempo, Vitesse, Velocidad, Velocità |
| download (action) | Stáhnout | Stiahnuť | Letöltés | Herunterladen | Télécharger | Descargar | Scarica | |
| offline (state) | offline / bez připojení | offline / bez pripojenia | offline | offline | hors ligne | sin conexión | offline | |
| queue | fronta | poradie | lejátszási sor | Warteschlange | file d’attente | cola | coda | settled 2026-11-22 (machine, unreviewed), like every row below. Add to queue: Přidat do fronty, Pridať do poradia, Hozzáadás a lejátszási sorhoz, Zur Warteschlange hinzufügen, Ajouter à la file d’attente, Añadir a la cola, Aggiungi alla coda. Queued: Ve frontě, V poradí, Sorban áll, In der Warteschlange, En file d’attente, En cola, In coda. sk never "front"; hu "sor" alone only in "Sorban áll" |
| Sleep (the timer's short button) | Spánek | Časovač | Elalvás | Schlaftimer | Minuteur | Temporizador | Timer | never "sleep mode" (sk Spánok, fr Veille). Czech is open: the Mac says Spánek, Music Usínání |
| Low Data Mode (iOS) | režim nízkých dat | režim nízkych dát | alacsony adatforgalmú mód | Datensparmodus | mode données faibles | modo de datos reducidos | modalità dati ridotti | Apple's names for the setting; Keychain likewise: klíčenka, kľúčenka, kulcskarika, Schlüsselbund, trousseau, llavero, portachiavi |
| favorite (verb) | Přidat do oblíbených | Pridať do obľúbených | Hozzáadás a kedvencekhez | Zu Favoriten hinzufügen | Ajouter aux favoris | Añadir a favoritos | Aggiungi ai preferiti | "Favorite Album" = Album zu Favoriten hinzufügen…; never "favorisieren". Unfavorite = the Favorites row's remove |
| sign-in (noun, the event) | přihlášení | prihlásenie | bejelentkezés | Anmeldung | connexion | inicio de sesión | accesso | "Signed in" in the account's activity is this noun |
| two-factor sign-in | dvoufázové přihlášení | dvojfázové prihlásenie | kétlépcsős bejelentkezés | Zwei-Faktor-Anmeldung | connexion en deux étapes | inicio de sesión en dos pasos | accesso in due passaggi | Two-factor code: Dvoufázový kód, Dvojfázový kód, Kétlépcsős kód, Zwei-Faktor-Code, Code de vérification en deux étapes, Código de verificación en dos pasos, Codice di verifica in due passaggi. Authenticator app: ověřovací aplikace, overovacia aplikácia, hitelesítő alkalmazás, Authentifizierungs-App, app d’authentification, app de autenticación, app di autenticazione |
| identify run | běh identifikace | beh identifikácie | azonosítási futtatás | Identifizierungsdurchlauf (Durchlauf) | analyse (Analyse d’identification) | ejecución (label: Identificación de biblioteca) | identificazione (label: Identificazione della libreria) | one run of Identify library. fr never "passe", es never "análisis", it never "esecuzione" / "analisi" |
| sure match | jistá shoda | istá zhoda | biztos egyezés | sicherer Treffer | correspondance sûre | coincidencia segura | corrispondenza sicura | |
| release / release group (MusicBrainz) | vydání / skupina vydání | vydanie / skupina vydaní | kiadás / kiadáscsoport | Veröffentlichung / Veröffentlichungsgruppe | parution / groupe de parutions | publicación / grupo de publicaciones | pubblicazione / gruppo di pubblicazioni | es never "edición" for a release: it is the edition's word |
| edition / recording | edice / nahrávka | edícia / nahrávka | kiadás / felvétel | Ausgabe / Aufnahme | édition / enregistrement | edición / grabación | edizione / registrazione | de never "Edition". hu uses kiadás for both release and edition: open |
| summary (AI, of an episode) | shrnutí | zhrnutie | összefoglaló | Zusammenfassung | résumé | resumen | riassunto | it never "riepilogo" for this (Riepilogo e conferma is a review step) |
| transcript | přepis | prepis | átirat | Transkript | transcription | transcripción | trascrizione | |
| sync | synchronizace / synchronizovat | synchronizácia / synchronizovať | szinkronizálás | Synchronisierung / synchronisieren | synchronisation / synchroniser | sincronización / sincronizar | sincronizzazione / sincronizza | lyrics "Synced" = časovaný (cs/sk), időzített |
| logs (the app's diagnostic log) | záznamy / Sdílet záznam | záznamy / Zdieľať záznam | naplók / Napló megosztása | Protokolle / Protokoll teilen | journaux / Partager le journal | registros / Compartir registro | registro / Condividi registro | the box's debug page says logy / Logs / log (technical, for developers): open |
| accent color | barva zvýraznění | farba zvýraznenia | kiemelőszín | Akzentfarbe | couleur d’accentuation | color de acento | colore di accento | |
| Smart rewind | Chytré přetočení | Inteligentné pretočenie | Okos visszatekerés | Intelligentes Zurückspulen | Retour intelligent | Retroceso inteligente | Riavvolgimento intelligente | |
| Keep ahead (stream buffer) | Uchovávat dopředu | Uchovávať dopredu | Előre tárolás | Vorausladen | Garder d’avance | Guardar por adelantado | Conserva in anticipo | |
| crossfade / gapless | prolínání / bez mezer | prelínanie / bez medzier | áttűnés / szünet nélküli | Überblenden / lückenlos | fondu enchaîné / sans blanc | fundido / sin pausas | dissolvenza incrociata / senza pause | |
| pin / unpin (keep on this Mac) | připnout / odepnout | pripnúť / odopnúť | kitűzés / kitűzés megszüntetése | anheften / lösen | épingler / désépingler | fijar / dejar de fijar | fissa / non fissare più | |
| mirror / evict (Mac files) | zrcadlit / uvolnit | zrkadliť / uvoľniť | tükrözés / felszabadítás | spiegeln / freigeben | mettre en miroir / libérer | replicar / liberar | rispecchia / libera | |
| folder | složka | priečinok | mappa | Ordner | dossier | carpeta | cartella | |
| private (listening stats, a shelf) | soukromé | súkromné | nem megosztott | privat | privé | privado | privato | hu "Privát" only in the "Make private" action's row above |
| Quoted (a job's price status) | Odhadovaná cena | Odhadovaná cena | Becsült ár | Geschätzt | Estimé | Estimado | Stima | was "Nabídnutá cena" in Czech, which says "offer" (see price estimate) |
| box (the device) | box | box | box | Box | box | box | box | the product's name for it; never zařízení / Gerät / appareil for the box itself |

## Address and style per language

| | Address | Button verbs | Quotes | Capitalization |
|---|---|---|---|---|
| Czech | formal (vykání) | infinitive: Přehrát, Smazat | „…“ | sentence case |
| Slovak | formal (vykanie), as the box | infinitive: Prehrať, Zmazať | „…“ | sentence case |
| Hungarian | formal (Ön, conjugated in the 3rd person, "Ön" written out only where needed), as the box; never "maga" | action noun: Lejátszás, Mentés, Törlés (as the box and Apple hu) | „…” | sentence case |
| German | informal (du, lowercase) | infinitive: Abspielen, Löschen | „…“ | nouns capitalized; otherwise sentence case, no Title Case |
| French | formal (vous) | infinitive: Lire, Supprimer | « … » with a narrow no-break space inside, and before ? ! : ; | sentence case for titles and buttons |
| Spanish (Spain) | informal (tú) | infinitive: Reproducir, Eliminar | «…» | sentence case; ¿…? and ¡…! |
| Italian | informal (tu) | imperative (tu): Riproduci, Elimina, Annulla | «…» | sentence case |

Why:

- **Czech, Slovak, Hungarian: formal.** Czech is decided above; the box already
  uses vykanie in Slovak and Ön in Hungarian, and Central European apps for
  a whole family (parents, grandparents) read as rude in the informal.
- **German: du.** Apple, Spotify and Audible address German users with du;
  Sie now reads as a bank or an insurer.
- **French: vous.** Apple's French UI uses vous; tu reads as a youth brand.
- **Spanish: tú, Spain.** Apple, Spotify and Audible use tú. The column is
  Spain Spanish (Ajustes, Añadir, ordenador); Latin America would need its own
  column (Configuración, Agregar), not a blend.
- **Italian: tu.** Apple and Spotify Italy use tu and the imperative on buttons.

Numbers and money: all six use a decimal comma, and the currency sign goes
after the amount with a space in most of them (`0,0635 $`). Never build these by
hand: pass the number and currency code to the OS or `Intl` formatter, which
also handles thousands separators (de `1.000`, fr `1 000` with a narrow
no-break space, hu/sk `1 000`).

Titles everywhere are sentence case: the English Title Case ("Up Next",
"Offline Mode") does not carry over.

## Open items

- **Native-speaker review of Slovak, Hungarian, German, French, Spanish and
  Italian** (written by machine 2026-11-22). Until each column is reviewed,
  translation passes use it as written and note any word they had to change
  here. Check the Apple-derived words against a device in that language.
- es: *pódcast* vs *podcast*; fr: the cover split (pochette / couverture). Every app writes
  *pódcast* today, as the column says.
- The rows from "queue" down were settled on 2026-11-22 by aligning the seven UIs; they are
  machine-made and unreviewed. Their Czech is the word the apps already show, except
  "Odhadovaná cena".

Windows (`audio2-win`) is not aligned yet; its deviations are listed in
`audio2-win/CATCH_UP_2026-09-24.md` §3.
