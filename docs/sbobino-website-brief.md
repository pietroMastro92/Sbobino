# Sbobino Website Brief for Claude Code

This file is written for **Claude Code**.

Its purpose is to help Claude Code understand **what Sbobino is**, **how it should be described**, and **how to build a public-facing website** that presents the app correctly and attractively.

If you are Claude Code and you are asked to create, redesign, or improve the Sbobino website, use this file as your product brief and messaging guide.

## Mission

Build a website that presents **Sbobino** as a serious desktop product for people who want to turn spoken content into something useful.

The website should feel:

- professional
- elegant
- clear
- modern
- slightly playful in a tasteful way

The website should **not** feel:

- generic AI slop
- overhyped marketing fluff
- developer-only
- gimmicky

The intended result is a product site in the spirit of polished feature-first desktop app websites, where the user understands the product quickly and sees why it is useful.

## Product Name

- Correct public name: **Sbobino**
- Do not present the app publicly as **Sbobino Tauri**
- "Tauri" is part of the technical stack, not the product name

## Meaning of the Name

The name comes from the Italian verb **sbobinare**, commonly used to mean transcribing the contents of a recording.

This origin is useful for branding because:

- it gives the name authenticity
- it links directly to the app’s core purpose
- it makes the brand memorable

Claude Code may mention this origin in the website, but should do so lightly and elegantly.

Good usage:

- “Sbobino comes from the Italian idea of turning recordings into usable text.”
- “The smart way to sbobinare lessons, meetings, and interviews.”

Bad usage:

- long linguistic explanations
- repetitive references in every section
- making the app sound like a joke

## What Sbobino Is

Sbobino is a desktop app for **Apple Silicon Macs** that transforms recordings into structured, reusable outputs.

At its core, Sbobino helps users:

- transcribe audio locally
- improve transcript readability
- generate summaries
- generate FAQs
- chat with transcript content
- manage speaker-aware segments
- export documents and subtitles

Sbobino is **not** just a raw transcription tool.

The right mental model is:

> A local-first desktop workspace that turns lessons, meetings, interviews, and voice notes into clear transcripts, summaries, and working documents.

## Primary Audiences

Claude Code should design the website for two main audiences.

### 1. Students

Students use Sbobino to:

- turn lectures into readable notes
- generate summaries after class
- review long spoken explanations
- create reusable study material from recordings

Key student framing:

- less time replaying recordings
- faster note recovery
- better study material from spoken content

### 2. Professionals

Professionals use Sbobino to:

- document meetings
- capture interviews
- review calls and brainstorming sessions
- turn recordings into structured outputs they can reuse

Key work framing:

- recording to summary
- meeting to document
- conversation to searchable knowledge

## Core Product Promise

Claude Code should keep one main promise at the center of the website:

> Sbobino shortens the distance between “I recorded something important” and “I now have something I can study, share, or work from.”

That is the single best framing for the site.

## Real Product Capabilities

Do not invent capabilities. Use only real product behaviors already present in the app.

### Local-first transcription

- transcribe local audio files in a desktop workflow
- show live progress during transcription
- manage active, queued, and completed jobs
- reopen previous transcripts from history

### Transcript improvement

- improve transcript wording with AI-assisted post-processing
- keep original and optimized transcript versions
- trim audio and create focused child transcripts

### Summaries and FAQs

- generate structured summaries from long recordings
- generate FAQ-style outputs from transcript content
- help users turn recordings into study-ready or work-ready material

### Chat with the transcript

- ask questions directly on transcript content
- recover facts and clarifications from long conversations
- use the transcript as an interactive knowledge source

### Speaker-aware review

- browse timestamped segments
- assign and manage speaker labels
- use local pyannote diarization when the runtime is installed

### Professional export

- export to `txt`, `docx`, `html`, `pdf`, `md`, `csv`, `json`, `srt`, and `vtt`
- export full transcripts and subtitle-oriented outputs
- include summaries and FAQs in exports when available

### Local setup and runtime

- public releases install the managed runtime on first launch
- the user should not need Homebrew or a separate transcription stack
- the app is designed for Apple Silicon Macs

### Faster warm starts

- after setup succeeds, later launches should feel fast
- the app should not repeat heavy setup checks on every opening

Claude Code may translate this into product language such as:

- “Set up once, then get back to work quickly.”
- “Built for a smoother desktop workflow after first launch.”

## Positioning Rules

Claude Code should position Sbobino as:

- a desktop productivity tool
- a spoken-content workflow app
- a local-first knowledge tool
- a serious study-and-work product

Claude Code should not position Sbobino as:

- a generic AI chat app
- a developer tool
- a cloud-first SaaS dashboard
- a mobile app
- a Windows app

## Tone of Voice

The website copy should sound:

- professional
- confident
- human
- polished
- a little charming

The copy should not sound:

- robotic
- sterile
- exaggerated
- full of empty AI buzzwords

### Good tone examples

- Turn long recordings into something you can actually use.
- From lecture to notes. From meeting to document.
- Sbobino does the tedious part of “sbobinare” so you can focus on understanding, editing, and sharing.
- A serious desktop app for spoken content, without the usual mess.
- Less rewinding. More understanding.

### Avoid these styles

- “Revolutionary AI platform”
- “Best-in-class solution”
- “Game-changing productivity engine”
- “Seamlessly leverage AI-driven synergies”

## Website Structure Claude Code Should Prefer

If Claude Code is building a landing page, prefer a structure like this:

### 1. Hero

The hero should communicate:

- what Sbobino is
- who it is for
- why it matters

Recommended hero content:

- headline about turning recordings into useful knowledge
- subheadline mentioning lessons, meetings, interviews, and voice notes
- primary CTA for download or release page
- secondary CTA for features or screenshots

### 2. Feature Grid

Feature cards or panels should cover:

- local-first transcription
- transcript cleanup
- summaries and FAQs
- transcript chat
- speaker-aware review
- professional export

### 3. Workflow Section

A 3-step or 4-step story works well:

1. import or record
2. transcribe and organize
3. summarize, ask, and refine
4. export and reuse

### 4. Audience Section

Separate messaging for:

- students
- professionals

This matters because the product is strongest when both audiences immediately see themselves in it.

### 5. Desktop Quality Section

Claude Code can highlight:

- Apple Silicon desktop app
- local runtime setup
- lightweight app bundle
- desktop workflow quality

### 6. Screenshots Section

Useful UI areas to feature:

- main transcript workspace
- summary/chat interface
- local models or setup area
- history/archive view
- export flow or segment-oriented editing

### 7. Final CTA

The final CTA should reinforce the core promise:

- recordings become usable material
- spoken content becomes study or work output
- less friction after lectures and meetings

## Design Direction

If Claude Code is asked to implement the site, the design should feel:

- premium but not flashy
- editorial but still product-oriented
- clean and intentional
- visually warm, not cold

Recommended design characteristics:

- strong typography
- restrained but expressive layout
- clear hierarchy
- well-presented screenshots
- subtle motion
- minimal visual noise

Avoid:

- generic SaaS cards everywhere
- random gradients with no product meaning
- cluttered feature overload
- dark-mode-only visual thinking unless explicitly requested

## Accuracy Constraints

Claude Code must not claim:

- Windows support
- iPhone or Android support
- “works everywhere”
- fully local AI for every optional remote provider feature
- Apple notarization/signing claims that are not actually true

Claude Code should stay grounded in what is true today:

- desktop app
- Apple Silicon focused
- local-first transcription runtime
- transcript improvement, summaries, FAQs, chat, speaker-aware review, and exports

## Metadata Copy Claude Code Can Reuse

### Short description

Sbobino is a desktop app for Apple Silicon Macs that turns lessons, meetings, interviews, and voice notes into clear transcripts, summaries, and useful documents.

### Slightly richer description

Sbobino helps students and professionals transform long recordings into clean transcripts, summaries, FAQs, and exportable documents inside a local-first desktop workflow.

## One-Liners Claude Code Can Reuse

- Sbobino turns lessons, meetings, and voice notes into transcripts, summaries, and documents you can actually use.
- The smart way to sbobinare recordings on your Mac.
- From spoken content to study notes and working documents.
- A desktop workspace for people who record first and organize later.
- Less rewinding. More understanding.

## Final Instruction for Claude Code

If you are Claude Code and you are asked to build the Sbobino website:

- use **Sbobino** as the product name everywhere
- treat this file as the product messaging brief
- optimize for students and professionals
- present the app as a real desktop product, not as a generic AI wrapper
- keep the tone professional, attractive, and slightly fun
- make the result feel polished enough for a serious public release

The final impression should be:

> “This looks like a real product, it solves a real problem, and I immediately understand why I would want it.”
