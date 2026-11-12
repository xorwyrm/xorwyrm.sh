// Everything the site says lives here.

export const site = {
  name: 'bit',
  domain: 'xorwyrm.sh',
  description: "bit's corner of the internet: tools, mods, and whatever i'm making lately.",
};

// the paragraph under the name. *stars* mark the words that stand out
export const intro =
  "21, from the us. i make *tools & mods* in rust, c++, kotlin and nix, and lately i'm deep in *security research*. nixos, hyprland, fish, and way too much deadlock.";

export const projects: { name: string; url: string; desc: string }[] = [
  {
    name: 'dropconfirm',
    url: 'https://modrinth.com/mod/drop-confirm',
    desc: 'minecraft mod that asks before you drop items, on every version from 1.14.4 up.',
  },
  {
    name: 'aurelia',
    url: 'https://github.com/xorwyrm/aurelia',
    desc: 'native jellyfin music client for android, ios and desktop, sharing one rust core.',
  },
  {
    name: 'draconis++',
    url: 'https://github.com/xorwyrm/draconisplusplus-monorepo',
    desc: 'fast system info tool and library. runs on windows, macos, linux, the bsds, haiku and serenityos.',
  },
  {
    name: 'denix-config',
    url: 'https://github.com/xorwyrm/denix-config',
    desc: 'my nixos, nix-darwin and home manager setup. every machine i own lives here.',
  },
];

export const links: { label: string; text: string; url: string }[] = [
  { label: 'github', text: 'xorwyrm', url: 'https://github.com/xorwyrm' },
  { label: 'x', text: '@xorwyrm', url: 'https://x.com/xorwyrm' },
  { label: 'modrinth', text: 'xorwyrm', url: 'https://modrinth.com/user/xorwyrm' },
];
