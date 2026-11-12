// Everything the site says lives in site.json, so the ssh version (ssh/) can read it too.
// This gives it its types.
import data from './site.json';

export const site: { name: string; domain: string; description: string } = data.site;

// the paragraph under the name. *stars* mark the words that stand out
export const intro: string = data.intro;

export const projects: { name: string; url: string; desc: string }[] = data.projects;

export const links: { label: string; text: string; url: string }[] = data.links;
