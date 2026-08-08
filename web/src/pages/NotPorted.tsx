// Step-0 placeholder.
//
// Each entry renders this until its page is ported. It exists so the build
// pipeline (four entries, one self-contained document each) is proven end to
// end before any UI depends on it — and so a document accidentally served
// early says what it is instead of showing a blank page.
//
// It deliberately renders one of each shadcn primitive we have added. Unused
// components tree-shake to nothing, so a placeholder that imported none of them
// would report a build size that no real page could achieve. Dialog is included
// specifically because it is the heaviest kind of Radix primitive (portal,
// focus trap, dismiss layer) and it is the honest upper bound.
import { useState } from 'react'
import { Markdown } from '@/components/Markdown'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'

export function NotPorted({ page }: { page: string }) {
  const [count, setCount] = useState(0)

  return (
    <main className="mx-auto max-w-2xl px-5 py-[10vh]">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            takomo <span className="font-mono text-primary">{page}</span>
            <Badge variant="secondary">not ported</Badge>
          </CardTitle>
        </CardHeader>
        <CardContent>
          <Markdown
            text={[
              `This page is **not ported yet**. The Rust binary still serves it from \`src/${page}.html\`.`,
              '',
              'What this document proves: four entries build, each as one self-contained file,',
              'with the shared library, the palette and the markdown renderer inlined — the',
              'renderer you are reading this through — plus the shadcn primitives below,',
              'themed from Takomo’s own tokens rather than shadcn defaults.',
            ].join('\n')}
          />
          <div className="mt-4 flex items-center gap-2">
            <Button onClick={() => setCount((c) => c + 1)}>clicked {count}×</Button>
            <Button variant="secondary">secondary</Button>
            <Button variant="destructive">destructive</Button>
            <Dialog>
              <DialogTrigger asChild>
                <Button variant="outline">dialog</Button>
              </DialogTrigger>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>Themed from tokens.css</DialogTitle>
                  <DialogDescription>
                    Every color here resolves through the Aquarelle palette, so light and dark
                    follow the OS with no toggle and no second set of values.
                  </DialogDescription>
                </DialogHeader>
              </DialogContent>
            </Dialog>
          </div>
        </CardContent>
      </Card>
    </main>
  )
}
