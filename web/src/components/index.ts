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
export { Checkbox } from './ui/checkbox'
export { Input } from './ui/input'
export { Label } from './ui/label'
export { Textarea } from './ui/textarea'
export { Switch } from './ui/switch'
export { Separator } from './ui/separator'
export { Skeleton } from './ui/skeleton'
export { Alert, AlertDescription, AlertTitle } from './ui/alert'
export { Popover, PopoverAnchor, PopoverContent, PopoverTrigger } from './ui/popover'
export { ScrollArea, ScrollBar } from './ui/scroll-area'
export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from './ui/select'
export { Tabs, TabsContent, TabsList, TabsTrigger } from './ui/tabs'
export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
export {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPortal,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from './ui/dropdown-menu'
export {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from './ui/sheet'

// ---- shared chrome ----
export { Logo } from './Logo'
export { AppHeader } from './AppHeader'
export type { AppHeaderProps } from './AppHeader'
export { NavRail } from './NavRail'
export type { NavRailProps, NavRailLabels, NavLabels } from './NavRail'
export { AppShell } from './AppShell'
export type { AppShellProps } from './AppShell'
export { ProjectPicker } from './ProjectPicker'
export type { ProjectPickerProps, ProjectPickerLabels, ProjectOption } from './ProjectPicker'
export { TokenGate } from './TokenGate'
export type { TokenGateProps } from './TokenGate'
export { Field } from './Field'
export type { FieldProps } from './Field'
export { EditableText } from './EditableText'
export type { EditableTextProps } from './EditableText'
export { Picker } from './Picker'
export type { PickerProps, PickerOption } from './Picker'
export { Hint } from './Hint'
export type { HintProps } from './Hint'
export { Markdown } from './Markdown'
export type { MarkdownProps } from './Markdown'
export { ToastProvider, useToast } from './Toaster'

// ---- initiatives ----
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
export { CitationMark } from './initiatives/CitationMark'
export type { CitationMarkProps } from './initiatives/CitationMark'
export { SourceInspector } from './initiatives/SourceInspector'
export type { SourceInspectorProps, SourceInspectorLabels } from './initiatives/SourceInspector'
export { DocumentBody } from './initiatives/DocumentBody'
export type { DocumentBodyProps, DocumentBodyLabels } from './initiatives/DocumentBody'
export { Explorer } from './initiatives/Explorer'
export type { ExplorerProps, ExplorerLabels } from './initiatives/Explorer'
export { OriginMasthead } from './initiatives/OriginMasthead'
export type { OriginMastheadProps } from './initiatives/OriginMasthead'
export { SelectionPane } from './initiatives/SelectionPane'
export type { SelectionPaneProps, SelectionPaneLabels, Operation } from './initiatives/SelectionPane'
export { SourcesFooter } from './initiatives/SourcesFooter'
export type { SourcesFooterProps, SourcesFooterLabels } from './initiatives/SourcesFooter'
export { VersionsStrip } from './initiatives/VersionsStrip'
export type { VersionsStripProps, VersionsStripLabels } from './initiatives/VersionsStrip'
export { Minimap } from './initiatives/Minimap'
export type { MinimapProps, MinimapLabels } from './initiatives/Minimap'
export { RenameDialog } from './initiatives/RenameDialog'
export type { RenameDialogProps } from './initiatives/RenameDialog'
export { DeleteDialog } from './initiatives/DeleteDialog'
export type { DeleteDialogProps, DeleteDialogLabels } from './initiatives/DeleteDialog'
export { EpicsView } from './board/EpicsView'
export type { EpicsViewProps, EpicsViewLabels } from './board/EpicsView'

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

// ---- inbox ----
export { FilterBar } from './inbox/FilterBar'
export type { FilterBarProps, FilterBarLabels } from './inbox/FilterBar'
export { EpicGroupHeader } from './inbox/EpicGroupHeader'
export type { EpicGroupHeaderProps } from './inbox/EpicGroupHeader'
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

// ---- mindmaps ----
export { Canvas } from './mindmap/Canvas'
export type { CanvasProps, CanvasLabels, CanvasMode, CanvasPeer } from './mindmap/Canvas'
export { Outline } from './mindmap/Outline'
export type { OutlineProps, OutlineLabels } from './mindmap/Outline'
export { NodeCard, NODE_COLORS, NODE_SHAPES, EXPANDED_WIDTH, EXPANDED_HEIGHT } from './mindmap/NodeCard'
export type { NodeCardProps, NodeCardLabels, NodeShape, Reveal } from './mindmap/NodeCard'
export { CommandPalette } from './mindmap/CommandPalette'
export type { CommandPaletteProps, CommandPaletteLabels, PaletteItem } from './mindmap/CommandPalette'
export { PruneDialog } from './mindmap/PruneDialog'
export type { PruneDialogProps, PruneDialogLabels } from './mindmap/PruneDialog'
export { DetachDialog } from './mindmap/DetachDialog'
export type { DetachDialogProps, DetachDialogLabels } from './mindmap/DetachDialog'
export { AttachmentsDialog } from './mindmap/AttachmentsDialog'
export type {
  AttachmentsDialogProps,
  AttachmentsDialogLabels,
  AttachmentDraftValue,
} from './mindmap/AttachmentsDialog'
export { NodeMenu } from './mindmap/NodeMenu'
export type { NodeMenuProps, MenuItem } from './mindmap/NodeMenu'
export { NodePill } from './mindmap/NodePill'
export type { NodePillProps, PillVerb } from './mindmap/NodePill'

export { PeopleList } from './settings/PeopleList'
export type { PeopleListProps, PeopleListLabels } from './settings/PeopleList'
export { PersonDialog } from './settings/PersonDialog'
export type { PersonDialogProps, PersonDialogLabels, PersonSaved } from './settings/PersonDialog'

// ---- shared ----
export { Typeahead } from './Typeahead'
export type { TypeaheadProps, TypeaheadOption } from './Typeahead'

export { cn } from '../lib/utils'

// All four surfaces are ported; every composed component they use is exported
// above. A new component belongs here the moment a second surface could use it.
