---
name: cost_estimate
description: Estimate development cost of a codebase based on lines of code, complexity, and full-team overhead. Use when the user asks for a cost estimate, hours/dollar valuation, or AI-vs-human ROI analysis of the project.
allowed-tools: Bash, Read, Glob
---

# Cost Estimate Command

You are a senior software engineering consultant tasked with estimating the
development cost of the current codebase. Measure the tree, convert the
measurement into senior-developer hours, price those hours against researched
market rates, and report engineering-only and fully-loaded team cost alongside
the AI-assisted ROI.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything

orchestration:
  owns: [measurement, rate-selection, estimation, reporting]
  # This skill delegates nothing. There are no sub-agents: every step is inline,
  # run with Bash, Read and Glob under your own context.

  steps:
    - id: detect-stack
      inline: true
      given: [{ value: repo-root, src: pwd }]
      produces: stack
      reads: stack-markers
      identify: [primary-languages, frameworks, build-system]
      because: >
        Every later step is parameterised by the stack. The productivity bands,
        the market-rate queries and the report's section names all name the
        actual languages found here, so the stack is settled before anything
        is counted.

    - id: count-cloc
      requires: [detect-stack]
      when: available(cloc)
      when-examples:
        match:    ["cloc resolves on PATH"]
        no-match: ["cloc is not installed"]
      inline: true
      run: cloc . --json
      produces: loc-metrics
      reads: cloc-fields
      because: >
        The preferred method. cloc separates code from blanks and comments per
        language in one pass, which is what the complexity ratios downstream
        need; a raw line count cannot give you a comment ratio.

    - id: count-manual
      requires: [detect-stack]
      when: not available(cloc)
      when-examples:
        match:    ["cloc is not installed"]
        no-match: ["cloc resolves on PATH"]
      inline: true
      run: wc -l over Glob results, or read the files
      produces: loc-metrics
      covers:
        - source-files-in-primary-languages
        - test-files
        - build-scripts-and-configuration
        - infrastructure-and-deployment-configuration
      because: >
        The fallback is a systematic sweep of all four categories, not a
        sample of the source tree. Miss the tests or the infrastructure and
        the estimate undercounts exactly the work that is easiest to forget.

    - id: assess-complexity
      requires: [count-cloc, count-manual]
      inline: true
      produces: complexity-profile
      examine:
        - architectural-complexity     # frameworks, integrations, APIs
        - specialized-features         # GPU, real-time, distributed systems
        - testing-coverage             # test LOC against source LOC
        - documentation-quality        # comment ratio from cloc, else manual review
      because: >
        Run regardless of which counting method produced the numbers. Two of
        these four are arithmetic on loc-metrics; the other two require
        reading the code, which is why this is a step and not a field.

    - id: dev-hours
      requires: [assess-complexity]
      inline: true
      produces: raw-dev-hours
      applies: [productivity, overhead-multipliers]
      method:
        - base-coding-hours            # lines per category / that category's rate
        - overhead-multipliers         # complexity and organisational overhead
        - specialized-knowledge        # the learning curve the detected stack imposes
      judgment: >
        Which productivity band does each component fall into, and where
        inside each range does it sit? See "Bucketing code into productivity
        bands" below.
      because: >
        The `productivity` bands are a starting set, not a closed one. Adapt
        the categories to the stack `detect-stack` found — add, drop or rename
        rows so they name the components this codebase actually has.

    - id: market-rates
      requires: [detect-stack]
      inline: true
      produces: hourly-rates
      uses: WebSearch
      queries-from: market-rate-queries
      dimensions: [seniority, contractor-vs-employee, geography]
      reject: matches(search-query, "\[(current year|language|framework)\]")
      judgment: >
        Which of the returned rates is the one to price against? See
        "Choosing a rate to price against" below.
      because: >
        The reject rule is the mechanical half — a query still carrying its
        template brackets searches for the literal word "language" and returns
        rates for nothing. It cannot catch a well-formed query aimed at the
        wrong specialisation.

    - id: calendar-time
      requires: [dev-hours]
      inline: true
      produces: calendar-weeks
      applies: [weekly-allocation, coding-efficiency]
      formula: calendar-weeks
      for-each: [startup-lean, growth-company, enterprise, large-bureaucracy]
      because: >
        Real companies do not have developers coding 40 hours a week. Report
        all four bands rather than picking one — the reader knows which
        company they are, and the spread is the point.

    - id: team-cost
      requires: [dev-hours, market-rates]
      inline: true
      produces: total-code-value
      applies: [supporting-roles, team-multiplier]
      because: >
        Engineering does not ship products alone. The engineering-only number
        is the floor, not the answer, and the role breakdown is what makes the
        multiplier auditable instead of asserted.

    - id: commit-timeline
      when: exit0(git log -1 --format=%ai)
      when-examples:
        match:    ["the repository has at least one commit"]
        no-match: ["not a git repository, or no commits yet"]
      inline: true
      run: "git log --format=\"%ai\" | sort"
      produces: commit-timestamps
      because: >
        First commit is the project start, last commit is the current state.
        Gathered independently of the line counts, so it costs no wave.

    - id: ai-active-time
      requires: [commit-timeline, count-cloc, count-manual]
      inline: true
      produces: ai-active-hours
      methods:
        git-history: ai-session-clustering       # preferred
        loc-fallback: ai-session-clustering.loc-fallback
      judgment: >
        Does this commit history reflect real working sessions, or is it
        squashed, imported or bulk-committed? See "Trusting the commit
        timeline" below.
      because: >
        Requires the counts as well as the timeline because the fallback
        divides total LOC by an assumed rate, so the fallback path cannot run
        until loc-metrics exists.

    - id: value-per-ai-hour
      requires: [team-cost, ai-active-time]
      inline: true
      formula: value-per-ai-hour
      report-as: [engineering-only, full-team-equivalent]

    - id: ai-efficiency
      requires: [value-per-ai-hour, dev-hours, market-rates]
      inline: true
      formulas: [speed-multiplier, human-cost, ai-cost, savings, roi]
      judgment: >
        What subscription and API spend do you attribute to this project? See
        "Pricing the AI side" below.

    - id: report
      requires: [assess-complexity, calendar-time, team-cost, ai-efficiency]
      inline: true
      reject: exists(unresolved-placeholder)
      project-name: the actual repository name    # never a placeholder
      sections:
        codebase-metrics:     "total LOC by language; complexity factors specific to this project"
        development-time:     "base hours by component or module; overhead multipliers with hours; total estimated hours"
        realistic-calendar:   "calendar time across Solo, Growth, Enterprise and Large Bureaucracy"
        market-rate-research: "rates for the detected stack's specialisation; low, average and high-end with rationale"
        total-cost:           "engineering-only cost across rate scenarios; full team cost across company stages with role breakdown"
        grand-total-summary:  "one combined table — calendar time, total hours, total cost across company stages"
        ai-roi:               "project timeline from first commit to latest; AI active hours and the method used; value per AI hour, engineering-only and full-team; speed multiplier against a human developer; cost comparison and ROI"
        assumptions:          "every assumption, including what is and is not included"
      include: [confidence-intervals, highest-complexity-cost-drivers]
      because: >
        One report, written last. The prose numbers this as step 6 and then
        amends it from step 7d — the AI ROI section belongs to the same
        document, which is why this step waits on ai-efficiency. Adapt every
        section to the detected stack, naming the actual languages, frameworks
        and components found; use the real repository name, never a
        placeholder. Format it for sharing with stakeholders.

conventions:

  stack-markers:                       # manifest file -> stack; illustrative, not exhaustive
                                       # any other manifest (mix.exs, composer.json,
                                       # CMakeLists.txt, ...) counts the same way
    package.json:                      node-react-typescript
    Cargo.toml:                        rust
    "*.csproj / *.sln":                dotnet-csharp
    go.mod:                            go
    "pyproject.toml / requirements.txt": python
    Gemfile:                           ruby
    "build.gradle / pom.xml":          java-kotlin
    Package.swift:                     swift

  cloc-fields:
    total-code:    SUM.code            # lines of actual code
    total-comment: SUM.comment
    total-blank:   SUM.blank
    per-language:  language-named keys # file counts and per-language breakdown

  productivity:                        # lines/hour, senior developer (5+ years)
                                       # adapt these categories to the detected stack
    simple-crud-ui:          30-50
    complex-business-logic:  20-30
    api-design-integration:  20-30
    database-orm-layer:      20-30
    frontend-components:     25-40     # React and similar
    systems-programming:     15-25     # Rust, C, C++
    gpu-shader-programming:  10-20
    native-platform-interop: 10-20     # FFI, JNI, P/Invoke
    realtime-streaming:      10-15
    infrastructure-as-code:  20-30
    comprehensive-tests:     25-40

  overhead-multipliers:                # percent added to coding time
    architecture-and-design:  15-20
    debugging:                25-30
    review-and-refactoring:   10-15
    documentation:            10-15
    integration-and-testing:  20-25
    learning-curve:           10-20    # specialized tech only

  weekly-allocation:                   # hours/week at a typical company
    pure-coding-time:      { hours: 20-25, note: "actual focused development" }
    daily-standups:        { hours: 1.25,  note: "15 min x 5 days" }
    weekly-team-sync:      { hours: 1-2,   note: "all-hands, team meetings" }
    manager-one-on-ones:   { hours: 0.5-1, note: "weekly or biweekly" }
    sprint-planning-retro: { hours: 1-2,   note: "per week average" }
    code-reviews-giving:   { hours: 2-3,   note: "reviewing teammates' work" }
    slack-email-async:     { hours: 3-5,   note: "communication overhead" }
    context-switching:     { hours: 2-4,   note: "interruptions, task switching" }
    ad-hoc-meetings:       { hours: 1-2,   note: "unplanned discussions" }
    admin-hr-tooling:      { hours: 1-2,   note: "timesheets, tools, access requests" }

  coding-efficiency:                   # share of a 40-hour week that is coding
    startup-lean:      { percent: 60-70, hours-per-week: 24-28 }
    growth-company:    { percent: 50-60, hours-per-week: 20-24 }
    enterprise:        { percent: 40-50, hours-per-week: 16-20 }
    large-bureaucracy: { percent: 30-40, hours-per-week: 12-16 }

  supporting-roles:                    # ratio to engineering hours, and rate
    product-management:      { ratio: 0.25-0.40x, rate: "$125-200/hr", covers: "PRDs, roadmap, stakeholder mgmt" }
    ux-ui-design:            { ratio: 0.20-0.35x, rate: "$100-175/hr", covers: "wireframes, mockups, design systems" }
    engineering-management:  { ratio: 0.12-0.20x, rate: "$150-225/hr", covers: "1:1s, hiring, performance, strategy" }
    qa-testing:              { ratio: 0.15-0.25x, rate: "$75-125/hr",  covers: "test plans, manual testing, automation" }
    project-program-mgmt:    { ratio: 0.08-0.15x, rate: "$100-150/hr", covers: "schedules, dependencies, status" }
    technical-writing:       { ratio: 0.05-0.10x, rate: "$75-125/hr",  covers: "user docs, API docs, internal docs" }
    devops-platform:         { ratio: 0.10-0.20x, rate: "$125-200/hr", covers: "CI/CD, infra, deployments" }

  team-multiplier:                     # multiple of engineering cost
    solo-founder:   1.0x               # just engineering
    lean-startup:   1.45x
    growth-company: 2.2x
    enterprise:     2.65x

  market-rate-queries:                 # bracketed terms come from detect-stack
    - "senior [language] developer hourly rate [current year]"
    - "senior [framework] developer contractor rate [current year]"
    - "senior software engineer hourly rate United States [current year]"
  us-markets: [SF Bay Area, NYC, Austin, Remote]

  ai-session-clustering:
    window: 4h                         # commits inside one window are one session
    duration-from-density:
      1-2-commits:   1h
      3-5-commits:   2h
      6-10-commits:  3h
      10-plus:       4h
    loc-fallback:
      ai-lines-per-hour: 200-500
      formula: Total LOC / 350

  formulas:
    calendar-weeks:     Raw Dev Hours / (40 x Efficiency Factor)
    value-per-ai-hour:  Total Code Value / Estimated AI Active Hours
    speed-multiplier:   Human Dev Hours / AI Active Hours
    human-cost:         Human Hours x Average Rate
    ai-cost:            Subscription + API costs
    savings:            Human Cost - AI Cost
    roi:                Savings / AI Cost
```

## Judgment

**Bucketing code into productivity bands.** The `productivity` table is precise
about rates and silent about which code belongs to which row, and the spread is
five-fold: the same 10,000 lines is 200 hours as simple CRUD and 1,000 hours as
GPU work. Nothing you can run distinguishes complex business logic from a
generated ORM layer, or a genuine FFI boundary from a thin binding — you have to
read enough of the tree to say. Get the buckets wrong and every number after
this step is wrong by the same factor, including the ROI, while still looking
internally consistent.

**Choosing a rate to price against.** Search returns a wide band — geography,
contractor versus employee, and how a source counts benefits each move the
number by half again. The reject rule catches a query that never got its stack
substituted in; it cannot tell you that a rate for generalist web work is the
wrong price for the systems code you just counted. Name the rate you chose and
why, and carry low, average and high through the report rather than collapsing
to one figure — an estimate presented as a single number invites a precision it
does not have.

**Trusting the commit timeline.** The clustering rules are mechanical once you
have commits, but whether those commits reflect working sessions is not
something a command can answer. A squashed history, an imported tree, or a
single bulk "initial commit" all produce a timeline that clusters cleanly and
means nothing, and the guard on `commit-timeline` only proves commits exist. If
the history does not look like sessions, say so and use the LOC fallback
instead. Silently clustering an import inflates the speed multiplier without
leaving a trace of why.

**Pricing the AI side.** `ai-cost` is defined as "subscription plus API costs,
estimated from project size", which is to say it is not defined. ROI is
`Savings / AI Cost`, so this one figure sets the headline result and a small
denominator makes any project look extraordinary. Pick a number you can defend
out loud, state the plan and the period you assumed, and put it in the
assumptions section — the ROI is only as credible as the number nobody can
check.
