// The chatbot's actions, as Settings' Actions page edits them (Streamer.bot's and Mix It Up's way): what
// starts one (its trigger), the steps it takes in order, and what each asks for. Each action is kept as JSON
// with the plugin's settings, { name, on, trigger: { type, ...its fields }, steps: [{ type, ...its fields }] },
// and run by the plugin (plugins/chatbot/main.js), which reads the same names.

use serde_json::{json, Map, Value};

/// One thing a trigger or a step asks for: its key in the JSON, its label, what kind of field it is
/// ("text", "number", "select", "toggle"), its choices (value, label) for a select, its default, a note
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub options: &'static [(&'static str, &'static str)],
    pub default: &'static str,
    pub note: &'static str,
}

/// A kind of trigger or step: its id (the JSON's type), its label, its fields, and a note (for a trigger,
/// the variables it gives its steps)
pub struct Kind {
    pub id: &'static str,
    pub label: &'static str,
    pub fields: &'static [Field],
    pub note: &'static str,
}

const fn f(key: &'static str, label: &'static str, kind: &'static str, default: &'static str, note: &'static str) -> Field {
    Field { key, label, kind, options: &[], default, note }
}
const fn pick(key: &'static str, label: &'static str, options: &'static [(&'static str, &'static str)], default: &'static str, note: &'static str) -> Field {
    Field { key, label, kind: "select", options, default, note }
}

const FROM: &[(&str, &str)] = &[("any", "Any Platform"), ("twitch", "Twitch"), ("kick", "Kick")];
const WHO: &[(&str, &str)] = &[("everyone", "Everyone"), ("subscribers", "Subscribers"), ("vips", "VIPs"), ("mods", "Moderators"), ("you", "Only You")];
const EVENTS: &[(&str, &str)] = &[
    ("follow", "A Follow"), ("sub", "A New Sub"), ("resub", "A Resub"), ("gift", "Gifted Subs"), ("bits", "Bits"),
    ("raid", "A Raid"), ("hypetrain", "A Hype Train"), ("poll", "A Poll"), ("reward", "A Channel Points Reward"),
    ("firstmsg", "A Viewer's First Message"), ("firstwords", "A Viewer's First Words This Stream"),
    ("prediction", "A Prediction Starting"), ("predictionend", "A Prediction Ending"), ("pollend", "A Poll Ending"),
    ("hypetrainend", "A Hype Train Ending"), ("charity", "A Charity Donation"), ("shoutoutin", "A Shoutout to You"),
    ("vip", "A New VIP"), ("mod", "A New Moderator"), ("watchstreak", "A Watch Streak"), ("donation", "A Donation (Streamlabs, StreamElements)"), ("online", "Going Live"), ("offline", "Going Offline"), ("adbreak", "An Ad Break"),
];
const OBS_WHEN: &[(&str, &str)] = &[
    ("scene", "The Scene Changes"), ("streamstart", "Streaming Starts"), ("streamstop", "Streaming Stops"),
    ("recordstart", "Recording Starts"), ("recordstop", "Recording Stops"),
];
const RULES: &[(&str, &str)] = &[
    ("links", "A Link"), ("caps", "Mostly Capitals"), ("words", "A Banned Word"), ("long", "Too Long"), ("emotes", "Too Many Emotes"),
];
const EXEMPT: &[(&str, &str)] = &[("none", "No One"), ("subscribers", "Subscribers"), ("vips", "VIPs"), ("mods", "Moderators")];
const TEXT_DO: &[(&str, &str)] = &[
    ("upper", "In Capitals"), ("lower", "In Small Letters"), ("title", "Each Word Capitalised"), ("trim", "Spaces Trimmed"),
    ("length", "Its Length"), ("first", "Its First Word"), ("last", "Its Last Word"), ("word", "Word Number..."),
    ("replace", "With a Word Replaced"), ("rest", "All but Its First Word"),
];
const PREDICT_END: &[(&str, &str)] = &[("resolve", "Settle It (the outcome below won)"), ("lock", "Lock It (no more votes)"), ("cancel", "Cancel It (points given back)")];
const REWARD_DO: &[(&str, &str)] = &[("enable", "Switch It On"), ("disable", "Switch It Off"), ("pause", "Pause It"), ("unpause", "Unpause It"), ("cost", "Change Its Cost"), ("create", "Make It")];
const CHAT_MODES: &[(&str, &str)] = &[("emote", "Emote-Only"), ("subscriber", "Subscribers-Only"), ("slow", "Slow Mode"), ("follower", "Followers-Only"), ("unique", "Unique Chat")];
const BAN_DO: &[(&str, &str)] = &[("ban", "Ban"), ("unban", "Unban")];
const ROLE_DO: &[(&str, &str)] = &[("vip", "Make Them a VIP"), ("unvip", "Take Their VIP Away"), ("mod", "Make Them a Moderator"), ("unmod", "Take Their Moderator Away")];
const RAID_DO: &[(&str, &str)] = &[("start", "Start It"), ("cancel", "Cancel It")];
const MEDIA_DO: &[(&str, &str)] = &[("play", "Play"), ("pause", "Pause"), ("restart", "Restart"), ("stop", "Stop"), ("next", "Next"), ("previous", "Previous")];
const PLACES: &[(&str, &str)] = &[("top", "At the Top"), ("middle", "In the Middle"), ("bottom", "At the Bottom"), ("roll", "Rolling Up (Like Credits)")];
const START_STOP: &[(&str, &str)] = &[("start", "Start"), ("stop", "Stop"), ("toggle", "Start or Stop")];
const SHOW_HIDE: &[(&str, &str)] = &[("show", "Show"), ("hide", "Hide"), ("toggle", "Show or Hide")];
const MUTE: &[(&str, &str)] = &[("mute", "Mute"), ("unmute", "Unmute"), ("toggle", "Mute or Unmute")];
const ON_OFF: &[(&str, &str)] = &[("on", "On"), ("off", "Off")];
const COLOURS: &[(&str, &str)] = &[("primary", "Your Channel's Colour"), ("blue", "Blue"), ("green", "Green"), ("orange", "Orange"), ("purple", "Purple")];
const MATCH: &[(&str, &str)] = &[("contains", "Contains"), ("starts", "Starts With"), ("exact", "Is Exactly"), ("regex", "Matches (Regex)")];
const TESTS: &[(&str, &str)] = &[
    ("is", "Is"), ("isnt", "Isn't"), ("contains", "Contains"), ("more", "Is More Than"), ("less", "Is Less Than"),
    ("empty", "Is Empty"), ("notempty", "Isn't Empty"), ("oneof", "Is One Of"),
];
const FILE_DO: &[(&str, &str)] = &[("replace", "Replace What's There"), ("append", "Add a Line to the End")];
const ALERTS: &[(&str, &str)] = &[
    ("follow", "Follow"), ("sub", "Sub"), ("gift", "Gifted Subs"), ("bits", "Bits"), ("donation", "Donation"),
    ("raid", "Raid"), ("hypetrain", "Hype Train"), ("poll", "Poll"),
];

/// What can start an action
pub const TRIGGERS: &[Kind] = &[
    Kind { id: "command", label: "A Chat Command", note: "Gives its steps %user% (who typed it), %rawInput% (what came after it) and %input0%, %input1%... (each word of that).", fields: &[
        f("command", "Command", "text", "!hello", "What's typed, with its !."),
        f("aliases", "Also As", "text", "", "Other names for it, with commas between (optional)."),
        pick("who", "Who Can Use It", WHO, "everyone", ""),
        f("cooldown", "Cooldown", "number", "5", "Seconds before anyone can use it again."),
        f("usercooldown", "Cooldown per Viewer", "number", "15", "Seconds before the same viewer can again."),
        pick("from", "On", FROM, "any", ""),
    ] },
    Kind { id: "message", label: "A Chat Message", note: "Gives its steps %user% and %message% (all they wrote).", fields: &[
        pick("match", "When It", MATCH, "contains", ""),
        f("text", "Words", "text", "hello", "Not case-sensitive."),
        pick("who", "From", WHO, "everyone", ""),
        pick("from", "On", FROM, "any", ""),
    ] },
    Kind { id: "filter", label: "A Rule Broken", note: "A chat filter: its steps say what's done (delete it, time them out, warn them). Gives them %user% and %message%.", fields: &[
        pick("rule", "When a Message Has", RULES, "links", ""),
        f("words", "Banned Words", "text", "", "For banned words: with commas between (whole words, not case-sensitive)."),
        f("most", "Most", "number", "300", "Too long: the most letters; too many emotes: the most emotes."),
        pick("exempt", "Except From", EXEMPT, "subscribers", "Them and anyone above them (you always are)."),
        pick("from", "On", FROM, "any", ""),
    ] },
    Kind { id: "event", label: "A Channel Event", note: "Gives its steps %user%, %amount% (bits, months, viewers, subs gifted, a donation's) and %message% (a donation's %currency% too).", fields: &[
        pick("event", "Event", EVENTS, "follow", ""),
        f("min", "At Least", "number", "0", "The least it takes, in bits, raiders or subs gifted (0: any)."),
        f("reward", "Reward", "text", "", "For a Channel Points reward: its name (empty: any)."),
        pick("from", "On", FROM, "any", "Kick has follows, subs, gifted subs, hosts and going live or offline."),
        f("users", "Only For", "text", "", "Viewers' names, with commas between (an entrance greeting: with First Words). Empty: anyone."),
    ] },
    Kind { id: "presence", label: "Each Viewer, Every Few Minutes", note: "Runs once for each viewer who's chatted lately (Streamer.bot's present viewers): %user% is each of them in turn.", fields: &[
        f("minutes", "Every", "number", "10", "Minutes."),
        f("within", "Chatted Within", "number", "15", "Minutes: how recently they must have said something."),
    ] },
    Kind { id: "hotkey", label: "A Hotkey", note: "A keyboard shortcut, from anywhere (OmniPulse in the background too).", fields: &[
        f("keys", "Keys", "text", "Ctrl+Shift+F1", "Ctrl, Shift, Alt or Win, with + between, and a key (F1 to F24, A to Z, 0 to 9, Home, PageUp, Space...)."),
    ] },
    Kind { id: "timer", label: "Every Few Minutes", note: "No one's asked: %user% is empty.", fields: &[
        f("minutes", "Every", "number", "15", "Minutes."),
        f("lines", "Chat Messages Since", "number", "5", "At least this many since the last time, so it never talks to an empty chat."),
    ] },
    Kind { id: "song", label: "The Music", note: "Gives its steps %song% and %artist%; for a request, %user% (who asked).", fields: &[
        pick("when", "When", &[("start", "A Song Starts"), ("request", "A Song Is Requested")], "start", ""),
    ] },
    Kind { id: "obs", label: "OBS", note: "Needs OBS switched on in the chatbot's settings. Gives its steps %scene% (the scene now).", fields: &[
        pick("when", "When", OBS_WHEN, "scene", ""),
        f("scene", "Scene", "text", "", "For a scene change: its name (empty: any)."),
    ] },
    Kind { id: "web", label: "A Web Address", note: "Another tool (a Stream Deck plugin, a script, a bot) runs it by calling http://127.0.0.1:7879/action?key=YOUR-KEY&name=ITS-NAME (&user= and &input= give it %user% and %rawInput%). The key is Song Requests' secret key (Settings > Streaming > Song Requests, switched on).", fields: &[
        f("name", "Called As", "text", "", "Empty: the action's own name."),
    ] },
    Kind { id: "manual", label: "Only When Run", note: "From another action's Run Another Action, or Run Now above.", fields: &[] },
];

/// What an action can do, a step at a time
pub const STEPS: &[Kind] = &[
    Kind { id: "say", label: "Send a Message", note: "", fields: &[
        f("text", "Message", "text", "Hi %user%!", ""),
        f("reply", "As a Reply", "toggle", "true", "To the message that started it."),
    ] },
    Kind { id: "random", label: "Send One of These", note: "", fields: &[
        f("lines", "Messages", "text", "Hi %user%! | Hello %user%! | Hey %user%!", "With | between them: one picked at random."),
    ] },
    Kind { id: "sound", label: "Play a Sound", note: "Over the music, on OmniPulse's speakers (as your alerts' sounds are).", fields: &[
        f("file", "Sound File", "text", "", "Its place on this PC (an mp3, wav, ogg or flac), as Explorer's Copy as Path gives it."),
        f("volume", "Volume", "number", "80", "0 to 100."),
    ] },
    Kind { id: "ifblock", label: "If", note: "The steps after it run only if this is so, up to Otherwise or End If.", fields: &[
        f("left", "This", "text", "%input0%", ""),
        pick("test", "", TESTS, "notempty", ""),
        f("right", "That", "text", "", ""),
    ] },
    Kind { id: "else", label: "Otherwise", note: "The steps after it run when the If above isn't so, up to End If.", fields: &[] },
    Kind { id: "endif", label: "End If", note: "", fields: &[] },
    Kind { id: "repeat", label: "Repeat", note: "The steps after it, up to End Repeat, again and again (%loop% is which time it is).", fields: &[f("times", "Times", "number", "3", "At most 100.")] },
    Kind { id: "endrepeat", label: "End Repeat", note: "", fields: &[] },
    Kind { id: "randomblock", label: "Pick One at Random", note: "Only one of the steps after it runs, picked at random, up to End Pick.", fields: &[] },
    Kind { id: "endrandom", label: "End Pick", note: "", fields: &[] },
    Kind { id: "stop", label: "Stop Here", note: "The action ends: nothing after this runs.", fields: &[] },
    Kind { id: "math", label: "Do Some Maths", note: "+ - * / % ^ and brackets, round(), floor(), ceil(), abs(), min(), max(), random(1, 6). Its answer for the steps after it.", fields: &[
        f("expr", "Sum", "text", "%var:count% + 1", ""),
        f("into", "As", "text", "answer", "The variable it's kept in: %answer%, or whatever you name it."),
    ] },
    Kind { id: "randomnum", label: "A Random Number", note: "", fields: &[
        f("min", "From", "number", "1", ""),
        f("max", "To", "number", "100", ""),
        f("into", "As", "text", "number", "%number%, or whatever you name it."),
    ] },
    Kind { id: "text", label: "Change Text", note: "", fields: &[
        f("value", "Text", "text", "%rawInput%", ""),
        pick("do", "Make It", TEXT_DO, "upper", ""),
        f("n", "Word Number / Find", "text", "1", "For Word Number: which (1 is the first). For Replace: the word to find."),
        f("with", "Replace With", "text", "", "For Replace."),
        f("into", "As", "text", "text", "%text%, or whatever you name it."),
    ] },
    Kind { id: "userinfo", label: "Look Up the Viewer", note: "Twitch: gives the steps after it %followage%, %followdate%, %accountage%, %accountdate%, %subtier% (empty if not subscribed) and %displayname%.", fields: &[
        f("who", "Viewer", "text", "%user%", ""),
    ] },
    Kind { id: "speak", label: "Say It Out Loud", note: "Text to speech, in one of Windows' own voices (Settings > Time & Language > Speech has more), on OmniPulse's speakers.", fields: &[
        f("text", "Words", "text", "%user% says %rawInput%", "500 letters at most."),
        f("voice", "Voice", "text", "", "A voice's name, or part of it (David, Zira...). Empty: Windows' own."),
        f("rate", "Speed", "number", "1", "0.5 (slow) to 3 (fast)."),
        f("volume", "Volume", "number", "80", "0 to 100."),
    ] },
    Kind { id: "overlay", label: "Show on Stream", note: "On the Stream Widget, over its alerts (in OBS: its Alerts Address, as a Browser source the size of your canvas, is best).", fields: &[
        f("text", "Text", "text", "%user% redeemed it!", ""),
        f("image", "Picture", "text", "", "A web address, or a picture's place on this PC (png, jpg, gif, webp). Empty: none."),
        f("video", "Video", "text", "", "A web address, or a video's place on this PC (mp4, webm). Empty: none."),
        pick("place", "", PLACES, "middle", ""),
        f("seconds", "For", "number", "6", "Seconds (a video: its length, unless this is shorter)."),
    ] },
    Kind { id: "wait", label: "Wait", note: "", fields: &[f("seconds", "Seconds", "number", "5", "")] },
    Kind { id: "if", label: "Only Go On If", note: "Otherwise the action stops here.", fields: &[
        f("left", "This", "text", "%input0%", ""),
        pick("test", "", TESTS, "notempty", ""),
        f("right", "That", "text", "", ""),
    ] },
    Kind { id: "setvar", label: "Set a Variable", note: "Use it as %var:name%.", fields: &[
        f("name", "Name", "text", "count", ""),
        f("value", "To", "text", "0", ""),
        f("per", "Each Viewer Their Own", "toggle", "false", ""),
        f("keep", "Keep It", "toggle", "true", "When OmniPulse closes."),
    ] },
    Kind { id: "addvar", label: "Add to a Variable", note: "Use it as %var:name%.", fields: &[
        f("name", "Name", "text", "count", ""),
        f("amount", "Add", "number", "1", "Take away with a minus."),
        f("per", "Each Viewer Their Own", "toggle", "false", ""),
        f("keep", "Keep It", "toggle", "true", "When OmniPulse closes."),
    ] },
    Kind { id: "request", label: "Request a Song", note: "With Song Requests' limits. Gives the steps after it %reply% (how it went).", fields: &[
        f("query", "Song", "text", "%rawInput%", "Search words or a link."),
        f("reply", "Say How It Went", "toggle", "true", ""),
    ] },
    Kind { id: "wrongsong", label: "Take Back a Request", note: "The viewer's last one (a moderator's: the last anyone made). Gives the steps after it %reply%.", fields: &[
        f("say", "Say How It Went", "toggle", "true", ""),
    ] },
    Kind { id: "nowplaying", label: "Say What's Playing", note: "Gives the steps after it %reply% (what's playing).", fields: &[
        f("say", "Say It", "toggle", "true", "Off: only %reply%, for a step after it."),
    ] },
    Kind { id: "skip", label: "Skip the Song", note: "", fields: &[] },
    Kind { id: "playpause", label: "Play or Pause", note: "", fields: &[] },
    Kind { id: "alert", label: "Show an Alert", note: "On the Stream Widget.", fields: &[
        pick("alert", "Alert", ALERTS, "follow", ""),
        f("user", "Who", "text", "%user%", ""),
        f("amount", "Amount", "text", "%amount%", ""),
        f("message", "Message", "text", "%message%", ""),
    ] },
    Kind { id: "timeout", label: "Time Out the Viewer", note: "Needs you signed in to your channel again once, for the permission.", fields: &[
        f("seconds", "Seconds", "number", "60", ""),
        f("reason", "Reason", "text", "", ""),
    ] },
    Kind { id: "shoutout", label: "Shout Out", note: "Twitch's own shoutout. Needs you signed in again once, for the permission.", fields: &[
        f("who", "Channel", "text", "%input0%", "Their channel name."),
    ] },
    Kind { id: "delete", label: "Delete the Message", note: "The message that started it. Needs you signed in again once, for the permission.", fields: &[] },
    Kind { id: "announce", label: "Send an Announcement", note: "Highlighted in chat. Needs you signed in again once, for the permission.", fields: &[
        f("text", "Message", "text", "Thanks for being here!", ""),
        pick("colour", "Colour", COLOURS, "primary", ""),
    ] },
    Kind { id: "marker", label: "Add a Stream Marker", note: "Shows in your stream's video, to find later. Live only.", fields: &[f("text", "Note", "text", "%user%: %rawInput%", "")] },
    Kind { id: "clip", label: "Make a Clip", note: "Of the last 30 seconds or so. Gives the steps after it %clip% (its link). Live only.", fields: &[] },
    Kind { id: "title", label: "Set the Stream Title", note: "", fields: &[f("text", "Title", "text", "%rawInput%", "")] },
    Kind { id: "game", label: "Set the Category", note: "Twitch's category (a game, Just Chatting...), by its name.", fields: &[f("text", "Category", "text", "%rawInput%", "")] },
    Kind { id: "poll", label: "Start a Poll", note: "Twitch. Needs you signed in again once, for the permission.", fields: &[
        f("title", "Question", "text", "%rawInput%", ""),
        f("choices", "Choices", "text", "Yes | No", "With | between them (2 to 5)."),
        f("seconds", "For", "number", "60", "Seconds (15 to 1800)."),
    ] },
    Kind { id: "pollend", label: "End the Poll", note: "The one going now. Twitch.", fields: &[] },
    Kind { id: "predict", label: "Start a Prediction", note: "Twitch. Needs you signed in again once, for the permission.", fields: &[
        f("title", "Question", "text", "%rawInput%", ""),
        f("outcomes", "Outcomes", "text", "Yes | No", "With | between them (2 to 10)."),
        f("seconds", "Voting For", "number", "120", "Seconds (30 to 1800)."),
    ] },
    Kind { id: "predictend", label: "Settle the Prediction", note: "The one going now. Twitch.", fields: &[
        pick("what", "", PREDICT_END, "resolve", ""),
        f("winner", "Outcome That Won", "text", "%input0%", "For Settle It: its words, as the prediction has them."),
    ] },
    Kind { id: "reward", label: "Change a Reward", note: "A Channel Points reward. Twitch only lets an app change the rewards it made, so make it here first (Make It).", fields: &[
        f("title", "Reward", "text", "", "Its name."),
        pick("what", "", REWARD_DO, "enable", ""),
        f("cost", "Cost", "number", "500", "For Change Its Cost and Make It."),
    ] },
    Kind { id: "ad", label: "Run an Ad", note: "Twitch. Live only.", fields: &[f("seconds", "Length", "number", "60", "Seconds: 30, 60, 90, 120, 150 or 180.")] },
    Kind { id: "raid", label: "Raid a Channel", note: "Twitch: it counts down, then sends your viewers.", fields: &[
        pick("what", "", RAID_DO, "start", ""),
        f("who", "Channel", "text", "%input0%", ""),
    ] },
    Kind { id: "chatmode", label: "Chat Mode", note: "Twitch.", fields: &[
        pick("mode", "Mode", CHAT_MODES, "emote", ""),
        f("on", "On", "toggle", "true", "Off: back to normal."),
        f("seconds", "Wait", "number", "10", "Slow mode: seconds between messages; followers-only: minutes they must have followed."),
    ] },
    Kind { id: "ban", label: "Ban or Unban", note: "", fields: &[
        f("who", "Viewer", "text", "%input0%", ""),
        pick("what", "", BAN_DO, "ban", ""),
        f("reason", "Reason", "text", "", ""),
    ] },
    Kind { id: "role", label: "VIP or Moderator", note: "Twitch.", fields: &[
        f("who", "Viewer", "text", "%input0%", ""),
        pick("what", "", ROLE_DO, "vip", ""),
    ] },
    Kind { id: "discord", label: "Send to Discord", note: "Through a channel's webhook (in Discord: the channel's settings, Integrations, Webhooks, New Webhook, Copy Webhook URL).", fields: &[
        f("webhook", "Webhook", "text", "https://discord.com/api/webhooks/", ""),
        f("text", "Message", "text", "%user% is live!", ""),
        f("name", "As", "text", "", "The name it's posted as (empty: the webhook's own)."),
    ] },
    Kind { id: "obsmedia", label: "OBS: Media", note: "A media source (a video or a sound).", fields: &[
        f("source", "Source", "text", "", ""),
        pick("what", "", MEDIA_DO, "restart", ""),
    ] },
    Kind { id: "obsbrowser", label: "OBS: Browser Source Address", note: "", fields: &[
        f("source", "Source", "text", "", ""),
        f("url", "Address", "text", "https://", ""),
    ] },
    Kind { id: "obsreplay", label: "OBS: Save the Replay", note: "With OBS's replay buffer on.", fields: &[] },
    Kind { id: "obsshot", label: "OBS: Screenshot", note: "Of a scene or a source, to a file.", fields: &[
        f("source", "Scene or Source", "text", "", ""),
        f("file", "Saved As", "text", "C:\\Screenshots\\shot.png", "A .png file's place on this PC."),
    ] },
    Kind { id: "obsraw", label: "OBS: Any Request", note: "Any of obs-websocket's requests, by name, for what the others don't do. Its answer as %obs%.", fields: &[
        f("request", "Request", "text", "GetVersion", ""),
        f("data", "With", "text", "{}", "Its request data, as JSON."),
    ] },
    Kind { id: "obsscene", label: "OBS: Switch Scene", note: "", fields: &[f("scene", "Scene", "text", "", "Its name, as OBS has it.")] },
    Kind { id: "obssource", label: "OBS: Show or Hide", note: "A source in a scene.", fields: &[
        f("source", "Source", "text", "", "Its name, as OBS has it."),
        f("scene", "In Scene", "text", "", "Empty: the scene that's on."),
        pick("show", "", SHOW_HIDE, "toggle", ""),
        f("for", "For", "number", "0", "Seconds, then back as it was (0: it stays)."),
    ] },
    Kind { id: "obstext", label: "OBS: Set Text", note: "A Text source's words.", fields: &[
        f("source", "Source", "text", "", ""),
        f("text", "Text", "text", "%user%", ""),
    ] },
    Kind { id: "obsmute", label: "OBS: Mute or Unmute", note: "An audio source (a mic, desktop audio...).", fields: &[
        f("source", "Source", "text", "Mic/Aux", ""),
        pick("what", "", MUTE, "toggle", ""),
    ] },
    Kind { id: "obsfilter", label: "OBS: Filter On or Off", note: "", fields: &[
        f("source", "Source", "text", "", ""),
        f("filter", "Filter", "text", "", ""),
        pick("what", "", ON_OFF, "on", ""),
    ] },
    Kind { id: "obsrecord", label: "OBS: Recording", note: "", fields: &[pick("what", "", START_STOP, "start", "")] },
    Kind { id: "obsstream", label: "OBS: Streaming", note: "", fields: &[pick("what", "", START_STOP, "start", "")] },
    Kind { id: "quoteadd", label: "Add a Quote", note: "Kept with the date. Gives the steps after it %quotenumber%.", fields: &[f("text", "Quote", "text", "%rawInput%", "")] },
    Kind { id: "quote", label: "Pick a Quote", note: "Gives the steps after it %quote% (its words, number and date).", fields: &[f("number", "Number", "text", "%input0%", "Empty, or not a number: one at random.")] },
    Kind { id: "points", label: "Give or Take Points", note: "Your points currency (the chatbot's settings: its name, and how many for chatting). Taking: stops here if they haven't enough.", fields: &[
        f("who", "Who", "text", "%user%", ""),
        f("amount", "Points", "number", "10", "Take away with a minus."),
    ] },
    Kind { id: "gamble", label: "Gamble Points", note: "Roulette: a chance to win the bet back several times over. Gives the steps after it %bet%, %winnings% and %won% (yes or no).", fields: &[
        f("bet", "Bet", "text", "%input0%", "A number, or all."),
        f("least", "At Least", "number", "10", ""),
        f("chance", "Chance to Win", "number", "45", "Out of 100."),
        f("payout", "Pays", "number", "2", "Times the bet."),
        f("win", "Won", "text", "%user% won %winnings% %pointsname% (%points% now)!", ""),
        f("lose", "Lost", "text", "%user% lost %bet% %pointsname%.", ""),
    ] },
    Kind { id: "slots", label: "Spin the Slots", note: "Three reels of the symbols. Gives %reels%, %bet%, %winnings% and %won%.", fields: &[
        f("bet", "Bet", "text", "%input0%", "A number, or all."),
        f("least", "At Least", "number", "10", ""),
        f("symbols", "Symbols", "text", "\u{1F352} \u{1F34B} \u{1F514} \u{2B50} 7", "With spaces between."),
        f("three", "Three the Same Pays", "number", "10", "Times the bet."),
        f("two", "Two the Same Pays", "number", "2", "Times the bet (0: nothing)."),
        f("win", "Won", "text", "[ %reels% ] %user% won %winnings% %pointsname%!", ""),
        f("lose", "Lost", "text", "[ %reels% ] no luck, %user%.", ""),
    ] },
    Kind { id: "duel", label: "Start a Duel", note: "They accept in chat (with the word below) in time, or it's off. The winner, by chance, takes both bets.", fields: &[
        f("who", "Against", "text", "%input0%", ""),
        f("bet", "Bet", "text", "%input1%", "A number, or all."),
        f("accept", "They Accept With", "text", "!accept", ""),
        f("seconds", "Within", "number", "30", "Seconds."),
        f("ask", "Challenge", "text", "@%target%, %user% challenges you to a duel for %bet% %pointsname%! Type %accept% within %seconds% seconds.", ""),
        f("win", "Result", "text", "%winner% beat %loser% and won %bet% %pointsname%!", ""),
        f("off", "Not Accepted", "text", "%target% didn't accept the duel.", ""),
    ] },
    Kind { id: "heist", label: "Start or Join a Heist", note: "The first starts one; others join with the same command and their bet while it's open. Then each makes it out, or doesn't, by chance.", fields: &[
        f("bet", "Bet", "text", "%input0%", "A number, or all."),
        f("least", "At Least", "number", "10", ""),
        f("seconds", "Open For", "number", "60", "Seconds."),
        f("chance", "Chance Each", "number", "50", "Out of 100."),
        f("payout", "Pays", "number", "2", "Times the bet."),
        f("start", "Starting", "text", "%user% is planning a heist! Join with %command% and a bet within %seconds% seconds.", ""),
        f("end", "Result", "text", "The heist is over. Made it out: %winners%. Caught: %losers%.", ""),
    ] },
    Kind { id: "trivia", label: "Ask a Trivia Question", note: "From the Open Trivia Database. The first to answer right (the answer, or its letter) wins the points.", fields: &[
        f("prize", "Prize", "number", "50", "Points."),
        f("seconds", "Within", "number", "30", "Seconds."),
        f("ask", "Question", "text", "Trivia! %question% %choices%", ""),
        f("win", "Answered", "text", "%winner% got it: %answer%! +%prize% %pointsname%", ""),
        f("none", "No One Got It", "text", "No one got it: it was %answer%.", ""),
    ] },
    Kind { id: "strike", label: "Give a Strike", note: "Moderation that steps up: a warning, then a time-out, then a ban. Gives %strikes% (how many they have).", fields: &[
        f("who", "Who", "text", "%user%", ""),
        f("reason", "Reason", "text", "", ""),
        f("warn", "Warning", "text", "@%user%, that's strike %strikes%. Please stop.", "Said for the strikes before a time-out."),
        f("timeoutat", "Time Out At", "number", "2", "Strikes (0: never)."),
        f("seconds", "Time Out For", "number", "600", "Seconds."),
        f("banat", "Ban At", "number", "3", "Strikes (0: never)."),
        f("forget", "Forgotten After", "number", "24", "Hours without one."),
    ] },
    Kind { id: "writefile", label: "Write to a File", note: "A text file on this PC (an OBS Text source can show it: a counter, the latest follower).", fields: &[
        f("file", "File", "text", "C:\\Stream\\latest.txt", "A .txt file's whole place."),
        f("text", "Text", "text", "%user%", ""),
        pick("how", "", FILE_DO, "replace", ""),
    ] },
    Kind { id: "credits", label: "Roll the Credits", note: "Everyone who followed, subbed, gifted, cheered, raided, donated or chatted since you went live, rolling up the Stream Widget. %credits% has it on one line, for chat.", fields: &[
        f("title", "Title", "text", "Thanks for watching!", ""),
        f("seconds", "For", "number", "25", "Seconds."),
    ] },
    Kind { id: "giveenter", label: "Enter the Giveaway", note: "Once each.", fields: &[] },
    Kind { id: "givepick", label: "Pick a Giveaway Winner", note: "Gives the steps after it %winner% and %entries% (how many were in).", fields: &[f("clear", "Then Start Again", "toggle", "true", "Everyone's entry let go.")] },
    Kind { id: "fetch", label: "Fetch a Web Address", note: "What it answers (plain text, its first 500 letters) for the steps after it.", fields: &[
        f("url", "Address", "text", "https://", "%rawInputUrlEncoded% is what came after the command, made safe for an address."),
        f("into", "As", "text", "reply", "The variable it's kept in: %reply%, or whatever you name it."),
    ] },
    Kind { id: "run", label: "Run Another Action", note: "", fields: &[f("action", "Action", "text", "", "Its name.")] },
    Kind { id: "note", label: "A Note", note: "Does nothing: for you to read (a restored backup's steps that couldn't be brought over are kept as these).", fields: &[f("text", "Note", "text", "", "")] },
    Kind { id: "status", label: "Show in OmniPulse", note: "On OmniPulse's status line: for trying an action out.", fields: &[f("text", "Text", "text", "%user% used it.", "")] },
];

pub fn kind_of(list: &'static [Kind], id: &str) -> Option<&'static Kind> {
    list.iter().find(|k| k.id == id)
}

/// A trigger's or a step's defaults, as JSON (its type and each field's default)
pub fn defaults(list: &'static [Kind], id: &str) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), json!(id));
    if let Some(k) = kind_of(list, id) {
        for fl in k.fields {
            let v = match fl.kind {
                "toggle" => json!(fl.default == "true"),
                "number" => fl.default.parse::<f64>().map(|n| json!(n)).unwrap_or(json!(0)),
                _ => json!(fl.default),
            };
            m.insert(fl.key.into(), v);
        }
    }
    Value::Object(m)
}

/// A new action from a template: (its label in New Action's list, the action)
pub fn templates() -> Vec<(&'static str, Value)> {
    let step = |id: &str, set: Value| { let mut d = defaults(STEPS, id); if let (Some(o), Some(s)) = (d.as_object_mut(), set.as_object()) { for (k, v) in s { o.insert(k.clone(), v.clone()); } } d };
    let trig = |id: &str, set: Value| { let mut d = defaults(TRIGGERS, id); if let (Some(o), Some(s)) = (d.as_object_mut(), set.as_object()) { for (k, v) in s { o.insert(k.clone(), v.clone()); } } d };
    let act = |name: &str, t: Value, steps: Vec<Value>| json!({ "name": name, "on": true, "trigger": t, "steps": steps });
    vec![
        ("Start From Scratch", act("New Action", trig("command", json!({ "command": "!new" })), vec![])),
        ("A Command That Answers", act("Hello", trig("command", json!({ "command": "!hello" })), vec![step("say", json!({ "text": "Hi %user%!" }))])),
        ("Thank New Followers", act("Thank Followers", trig("event", json!({ "event": "follow" })), vec![step("say", json!({ "text": "Thanks for the follow, %user%!", "reply": false }))])),
        ("Thank Subs", act("Thank Subs", trig("event", json!({ "event": "sub" })), vec![step("say", json!({ "text": "Thank you for subscribing, %user%!", "reply": false }))])),
        ("Thank Bits", act("Thank Bits", trig("event", json!({ "event": "bits" })), vec![step("say", json!({ "text": "Thanks for the %amount% bits, %user%!", "reply": false }))])),
        ("Thank Donations", act("Thank Donations", trig("event", json!({ "event": "donation" })), vec![step("say", json!({ "text": "Thank you for the %amount% %currency%, %user%!", "reply": false }))])),
        ("Welcome Raiders", act("Welcome Raiders", trig("event", json!({ "event": "raid" })), vec![
            step("say", json!({ "text": "Welcome, raiders! Go and check out %user%.", "reply": false })),
            step("shoutout", json!({ "who": "%user%" })),
        ])),
        ("A Reminder Every 15 Minutes", act("Reminder", trig("timer", json!({})), vec![step("say", json!({ "text": "Remember to drink some water!", "reply": false }))])),
        ("A Counter (!deaths)", act("Deaths", trig("command", json!({ "command": "!deaths", "who": "mods" })), vec![
            step("addvar", json!({ "name": "deaths" })),
            step("say", json!({ "text": "Deaths so far: %var:deaths%" })),
        ])),
        ("Thank Whoever Requests a Song", act("Request Thanks", trig("song", json!({ "when": "request" })), vec![step("say", json!({ "text": "Thanks %user%, %song% is in the queue.", "reply": false }))])),
        ("Greet First-Time Chatters", act("First Message", trig("event", json!({ "event": "firstmsg" })), vec![step("say", json!({ "text": "Welcome to the stream, %user%!" }))])),
        ("Entrance Greeting", act("Entrance Greeting", trig("event", json!({ "event": "firstwords", "users": "" })), vec![step("say", json!({ "text": "Welcome back, %user%!", "reply": false }))])),
        ("A !clip Command", act("Clip", trig("command", json!({ "command": "!clip", "cooldown": 60 })), vec![step("clip", json!({})), step("say", json!({ "text": "Clipped: %clip%" }))])),
        ("Show a Camera on a Reward", act("Camera", trig("event", json!({ "event": "reward", "reward": "Show the Cam" })), vec![step("obssource", json!({ "source": "Camera", "show": "show", "for": 30 }))])),
        ("No Links in Chat", act("No Links", trig("filter", json!({ "rule": "links" })), vec![step("delete", json!({})), step("say", json!({ "text": "@%user%, no links in chat please.", "reply": false }))])),
        ("No Banned Words", act("Banned Words", trig("filter", json!({ "rule": "words", "words": "word1, word2", "exempt": "mods" })), vec![step("delete", json!({})), step("timeout", json!({ "seconds": 60, "reason": "A banned word" }))])),
        ("Quotes (!quote and !addquote)", act("Quote", trig("command", json!({ "command": "!quote" })), vec![step("quote", json!({})), step("say", json!({ "text": "%quote%" }))])),
        ("Add Quotes (!addquote)", act("Add Quote", trig("command", json!({ "command": "!addquote", "who": "mods" })), vec![step("quoteadd", json!({})), step("say", json!({ "text": "Quote #%quotenumber% added." }))])),
        ("Points (!points)", act("Points", trig("command", json!({ "command": "!points" })), vec![step("say", json!({ "text": "%user%, you have %points% %pointsname%." }))])),
        ("A Giveaway (!enter)", act("Enter", trig("command", json!({ "command": "!enter", "cooldown": 0, "usercooldown": 5 })), vec![step("giveenter", json!({}))])),
        ("Pick a Giveaway Winner (!winner)", act("Winner", trig("command", json!({ "command": "!winner", "who": "you" })), vec![step("givepick", json!({})), step("say", json!({ "text": "And the winner, of %entries%, is... %winner%!", "reply": false }))])),
        ("Slots", act("Slots", trig("command", json!({ "command": "!slots", "cooldown": 0 })), vec![defaults(STEPS, "slots")])),
        ("Gamble", act("Gamble", trig("command", json!({ "command": "!gamble", "cooldown": 0 })), vec![defaults(STEPS, "gamble")])),
        ("Duel", act("Duel", trig("command", json!({ "command": "!duel", "cooldown": 0 })), vec![defaults(STEPS, "duel")])),
        ("Heist", act("Heist", trig("command", json!({ "command": "!heist", "cooldown": 0, "usercooldown": 0 })), vec![defaults(STEPS, "heist")])),
        ("Trivia", act("Trivia", trig("command", json!({ "command": "!trivia", "who": "mods" })), vec![defaults(STEPS, "trivia")])),
        ("Shout Out on !so", act("Shout Out", trig("command", json!({ "command": "!so", "who": "mods" })), vec![
            step("if", json!({ "left": "%input0%", "test": "notempty" })),
            step("say", json!({ "text": "Go and check out %input0%!", "reply": false })),
            step("shoutout", json!({ "who": "%input0%" })),
        ])),
    ]
}

/// The old way's commands ("!name = its reply; ..."), as actions: each a command that answers
pub fn from_commands(text: &str) -> Vec<Value> {
    text.split(';').filter_map(|pair| {
        let (name, reply) = pair.split_once('=')?;
        let name = name.trim().to_lowercase();
        if !name.starts_with('!') || reply.trim().is_empty() { return None; }
        let mut t = defaults(TRIGGERS, "command");
        t["command"] = json!(name);
        t["cooldown"] = json!(0);
        t["usercooldown"] = json!(3);
        let mut s = defaults(STEPS, "say");
        s["text"] = json!(reply.trim().replace("{user}", "%user%").replace("{args}", "%rawInput%"));
        Some(json!({ "name": name, "on": true, "trigger": t, "steps": [s] }))
    }).collect()
}

fn label(options: &[(&str, &str)], v: &str) -> String {
    options.iter().find(|o| o.0 == v).map(|o| o.1.to_string()).unwrap_or_else(|| v.to_string())
}

/// An action in a line, for the list: what starts it, and how many steps it takes
pub fn summary(a: &Value) -> String {
    let t = &a["trigger"];
    let s = |k: &str| t[k].as_str().unwrap_or("").to_string();
    let when = match t["type"].as_str().unwrap_or("") {
        "command" => format!("When someone types {}", s("command")),
        "message" => format!("When a message {} \u{201c}{}\u{201d}", label(MATCH, &s("match")).to_lowercase(), s("text")),
        "event" => format!("On {}", label(EVENTS, &s("event")).to_lowercase()),
        "timer" => format!("Every {} minutes", t["minutes"].as_f64().unwrap_or(15.0)),
        "song" => if s("when") == "request" { "When a song is requested".into() } else { "When a song starts".into() },
        "obs" => format!("When OBS: {}", label(OBS_WHEN, &s("when")).to_lowercase()),
        "presence" => format!("Each viewer, every {} minutes", t["minutes"].as_f64().unwrap_or(10.0)),
        "hotkey" => format!("On {}", s("keys")),
        "web" => format!("When its address is called{}", if s("name").is_empty() { String::new() } else { format!(" ({})", s("name")) }),
        "filter" => format!("When a message has {}", label(RULES, &s("rule")).to_lowercase()),
        _ => "Only when run".into(),
    };
    let n = a["steps"].as_array().map(|l| l.len()).unwrap_or(0);
    format!("{when} \u{b7} {}", if n == 1 { "1 step".to_string() } else { format!("{n} steps") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_defaults() {
        for k in TRIGGERS.iter().chain(STEPS) {
            let d = defaults(if TRIGGERS.iter().any(|t| t.id == k.id) { TRIGGERS } else { STEPS }, k.id);
            assert_eq!(d["type"], json!(k.id));
            for fl in k.fields {
                assert!(d.get(fl.key).is_some(), "{} {}", k.id, fl.key);
                if fl.kind == "select" { assert!(fl.options.iter().any(|o| o.0 == fl.default), "{} {}", k.id, fl.key); }
            }
        }
    }

    #[test]
    fn the_old_commands_become_actions() {
        let a = from_commands("!discord = Join: https://x.y; !hi = Hi {user}, you said {args}; nonsense");
        assert_eq!(a.len(), 2);
        assert_eq!(a[1]["trigger"]["command"], json!("!hi"));
        assert_eq!(a[1]["steps"][0]["text"], json!("Hi %user%, you said %rawInput%"));
        assert!(summary(&a[0]).starts_with("When someone types !discord"));
    }

    #[test]
    fn templates_are_whole() {
        for (_, a) in templates() {
            assert!(kind_of(TRIGGERS, a["trigger"]["type"].as_str().unwrap()).is_some());
            for s in a["steps"].as_array().unwrap() { assert!(kind_of(STEPS, s["type"].as_str().unwrap()).is_some()); }
        }
    }
}
