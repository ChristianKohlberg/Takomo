// The library barrel — the entry point of the `build:lib` output, and therefore
// the surface a design system consumes (see vite.lib.config.ts).
//
// A component only belongs here once it takes everything it needs as props. If
// exporting one requires dragging page state along, that is the signal to fix
// the component, not to widen this file.
//
// The shadcn primitives belong here too, and not as a technicality: shadcn is
// copy-in, so `src/components/ui/*` is OUR source, themed from tokens.css. A
// design agent composing a Takomo screen needs the same Button and Card the app
// uses — handing it un-themed upstream defaults would produce designs that do
// not match anything that ships.

// The stylesheet ships WITH the library. Every component here styles itself with
// Tailwind utilities resolved from Takomo's own tokens, so shipping the JS alone
// would hand a consumer unstyled boxes. The app imports the same file directly;
// the duplicate import is deduped.
import '../styles/globals.css'
import '../styles/markdown.css'

// ---- primitives (shadcn, themed from tokens.css) ----
export { Button, buttonVariants } from './ui/button'
export { Badge, badgeVariants } from './ui/badge'
export {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from './ui/card'
export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
} from './ui/dialog'
export { Input } from './ui/input'
export { Label } from './ui/label'
export { Textarea } from './ui/textarea'

// ---- shared chrome ----
export { Logo } from './Logo'
export { AppHeader } from './AppHeader'
export type { AppHeaderProps, NavLabels } from './AppHeader'
export { TokenGate } from './TokenGate'
export type { TokenGateProps } from './TokenGate'
export { Field } from './Field'
export type { FieldProps } from './Field'
export { EditableText } from './EditableText'
export type { EditableTextProps } from './EditableText'
export { Markdown } from './Markdown'
export type { MarkdownProps } from './Markdown'
export { ToastProvider, useToast } from './Toaster'

// ---- initiatives ----
export { InitiativeRow } from './initiatives/InitiativeRow'
export type { InitiativeRowProps } from './initiatives/InitiativeRow'
export { StatusBadge } from './initiatives/StatusBadge'
export type { StatusBadgeProps } from './initiatives/StatusBadge'
export { RollupStrip } from './initiatives/RollupStrip'
export type { RollupStripProps } from './initiatives/RollupStrip'
export { EntryCard } from './initiatives/EntryCard'
export type { EntryCardProps } from './initiatives/EntryCard'
export { Composer } from './initiatives/Composer'
export type { ComposerProps, Draft, PickedFile } from './initiatives/Composer'
export { CreateDialog } from './initiatives/CreateDialog'
export type { CreateDialogProps } from './initiatives/CreateDialog'

// ---- schedules ----
export { ScheduleCard } from './schedules/ScheduleCard'
export type { ScheduleCardProps, ScheduleCardLabels } from './schedules/ScheduleCard'
export { OccurrenceStrip } from './schedules/OccurrenceStrip'
export type { OccurrenceStripProps } from './schedules/OccurrenceStrip'
export { CreateScheduleDialog } from './schedules/CreateScheduleDialog'
export type { CreateScheduleDialogProps } from './schedules/CreateScheduleDialog'

// ---- board ----
export { TicketCard } from './board/TicketCard'
export type { TicketCardProps } from './board/TicketCard'
export { Column } from './board/Column'
export type { ColumnProps } from './board/Column'
export { DetailPanel } from './board/DetailPanel'
export type { DetailPanelProps, DetailPanelLabels, OpenQuestions } from './board/DetailPanel'
export { AskDrawer } from './board/AskDrawer'
export type { AskDrawerProps, AskFields } from './board/AskDrawer'
export { InboxDrawer } from './board/InboxDrawer'
export type { InboxDrawerProps, InboxDrawerLabels } from './board/InboxDrawer'
export { SettingsSheet } from './board/SettingsSheet'
export type { SettingsSheetProps } from './board/SettingsSheet'

// ---- inbox ----
export { FolderRail } from './inbox/FolderRail'
export type { FolderRailProps } from './inbox/FolderRail'
export { QuestionRow } from './inbox/QuestionRow'
export type { QuestionRowProps } from './inbox/QuestionRow'
export { ReadingPane } from './inbox/ReadingPane'
export type { ReadingPaneProps, ReadingPaneLabels } from './inbox/ReadingPane'
export { AnswerArea } from './inbox/AnswerArea'
export type { AnswerAreaProps, AnswerAreaLabels } from './inbox/AnswerArea'
export { UndoSnackbar } from './inbox/UndoSnackbar'
export type { UndoSnackbarProps } from './inbox/UndoSnackbar'
export { AnswerLinkDialog } from './inbox/AnswerLinkDialog'
export type { AnswerLinkDialogProps } from './inbox/AnswerLinkDialog'

// ---- shared ----
export { Typeahead } from './Typeahead'
export type { TypeaheadProps, TypeaheadOption } from './Typeahead'

export { cn } from '../lib/utils'

// All four surfaces are ported; every composed component they use is exported
// above. A new component belongs here the moment a second surface could use it.
