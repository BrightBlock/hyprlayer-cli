---
description: Estimate development cost of a codebase based on lines of code and complexity
subtask: false
---

# Cost Estimate Command

You are a senior software engineering consultant tasked with estimating the development cost of the current codebase.

## Step 1: Analyze the Codebase

### 1a: Detect Technology Stack

First, detect the technology stack by examining project files:
- Look for `package.json` (Node/React/TypeScript), `Cargo.toml` (Rust), `*.csproj`/`*.sln` (.NET/C#), `go.mod` (Go), `pyproject.toml`/`requirements.txt` (Python), `Gemfile` (Ruby), `build.gradle`/`pom.xml` (Java/Kotlin), `Package.swift` (Swift), etc.
- Identify the primary language(s), frameworks, and build system

### 1b: Count Lines of Code

**Preferred Method: Use `cloc` (if installed)**

First, check if `cloc` is available by running:
```bash
command -v cloc
```

If `cloc` is installed, use it for accurate line counts:
```bash
cloc . --json
```

This provides:
- Lines of code by language (excluding blanks and comments)
- Blank line counts
- Comment line counts  
- File counts per language

Parse the JSON output to extract metrics. Key fields:
- `SUM.code` - Total lines of actual code
- `SUM.comment` - Total comment lines
- `SUM.blank` - Total blank lines
- Per-language breakdowns in the language-named keys

**Fallback Method: Manual Counting (if `cloc` unavailable)**

If `cloc` is not installed, use the Glob and Read tools to systematically review:
- All source files in the primary language(s)
- All test files
- Build scripts and configuration files
- Infrastructure/deployment configuration

Count lines manually using `wc -l` on glob results or by reading files.

### 1c: Assess Complexity

Regardless of which counting method was used, read the codebase to understand:
- Architectural complexity (frameworks, integrations, APIs)
- Advanced or specialized features (GPU programming, real-time systems, distributed systems, etc.)
- Testing coverage (compare test LOC to source LOC)
- Documentation quality (comment ratio from cloc, or manual review)

## Step 2: Calculate Development Hours

Based on industry standards for a **senior developer** (5+ years experience):

**Hourly Productivity Estimates** (adapt categories to the detected stack):
- Simple CRUD/UI code: 30-50 lines/hour
- Complex business logic: 20-30 lines/hour
- API design & integration: 20-30 lines/hour
- Database/ORM layer: 20-30 lines/hour
- Frontend components (React, etc.): 25-40 lines/hour
- Systems programming (Rust, C/C++): 15-25 lines/hour
- GPU/shader programming: 10-20 lines/hour
- Native platform interop (FFI, JNI, P/Invoke): 10-20 lines/hour
- Real-time/streaming processing: 10-15 lines/hour
- Infrastructure as code: 20-30 lines/hour
- Comprehensive tests: 25-40 lines/hour

**Additional Time Factors**:
- Architecture & design: +15-20% of coding time
- Debugging & troubleshooting: +25-30% of coding time
- Code review & refactoring: +10-15% of coding time
- Documentation: +10-15% of coding time
- Integration & testing: +20-25% of coding time
- Learning curve (new frameworks): +10-20% for specialized tech

**Calculate total hours** considering:
1. Base coding hours (lines of code / productivity rate per category)
2. Multipliers for complexity and overhead
3. Specialized knowledge required for the detected stack

## Step 3: Research Market Rates

Use WebSearch to find current hourly rates for developers with the detected stack's specialization:
- Senior developers with the primary language/framework
- Contractors vs. employees
- Geographic variations (US markets: SF Bay Area, NYC, Austin, Remote)

Adapt search queries to the detected stack, e.g.:
- "senior [language] developer hourly rate [current year]"
- "senior [framework] developer contractor rate [current year]"
- "senior software engineer hourly rate United States [current year]"

## Step 4: Calculate Organizational Overhead

Real companies don't have developers coding 40 hours/week. Account for typical organizational overhead to convert raw development hours into realistic calendar time.

**Weekly Time Allocation for Typical Company**:

| Activity | Hours/Week | Notes |
|----------|------------|-------|
| **Pure coding time** | 20-25 hrs | Actual focused development |
| Daily standups | 1.25 hrs | 15 min x 5 days |
| Weekly team sync | 1-2 hrs | All-hands, team meetings |
| 1:1s with manager | 0.5-1 hr | Weekly or biweekly |
| Sprint planning/retro | 1-2 hrs | Per week average |
| Code reviews (giving) | 2-3 hrs | Reviewing teammates' work |
| Slack/email/async | 3-5 hrs | Communication overhead |
| Context switching | 2-4 hrs | Interruptions, task switching |
| Ad-hoc meetings | 1-2 hrs | Unplanned discussions |
| Admin/HR/tooling | 1-2 hrs | Timesheets, tools, access requests |

**Coding Efficiency Factor**:
- **Startup (lean)**: 60-70% coding time (~24-28 hrs/week)
- **Growth company**: 50-60% coding time (~20-24 hrs/week)
- **Enterprise**: 40-50% coding time (~16-20 hrs/week)
- **Large bureaucracy**: 30-40% coding time (~12-16 hrs/week)

**Calendar Weeks Calculation**:
```
Calendar Weeks = Raw Dev Hours / (40 x Efficiency Factor)
```

## Step 5: Calculate Full Team Cost

Engineering doesn't ship products alone. Calculate the fully-loaded team cost including all supporting roles.

**Supporting Role Ratios** (expressed as ratio to engineering hours):

| Role | Ratio to Eng Hours | Typical Rate | Notes |
|------|-------------------|--------------|-------|
| Product Management | 0.25-0.40x | $125-200/hr | PRDs, roadmap, stakeholder mgmt |
| UX/UI Design | 0.20-0.35x | $100-175/hr | Wireframes, mockups, design systems |
| Engineering Management | 0.12-0.20x | $150-225/hr | 1:1s, hiring, performance, strategy |
| QA/Testing | 0.15-0.25x | $75-125/hr | Test plans, manual testing, automation |
| Project/Program Management | 0.08-0.15x | $100-150/hr | Schedules, dependencies, status |
| Technical Writing | 0.05-0.10x | $75-125/hr | User docs, API docs, internal docs |
| DevOps/Platform | 0.10-0.20x | $125-200/hr | CI/CD, infra, deployments |

**Full Team Multiplier**:
- **Solo/Founder**: 1.0x (just engineering)
- **Lean Startup**: ~1.45x engineering cost
- **Growth Company**: ~2.2x engineering cost
- **Enterprise**: ~2.65x engineering cost

## Step 6: Generate Cost Estimate

Provide a comprehensive estimate. Adapt all sections to the detected technology stack — use the actual languages, frameworks, and components found in the codebase. Do not use placeholder project names; use the actual repository name.

The report should include these sections:

### Codebase Metrics
- Total lines of code broken down by language
- Complexity factors specific to this project

### Development Time Estimate
- Base development hours by component/module
- Overhead multipliers with hours
- Total estimated hours

### Realistic Calendar Time
- Table showing calendar time across company types (Solo, Growth, Enterprise, Large Bureaucracy)

### Market Rate Research
- Rates specific to the detected stack's specialization
- Low, average, and high-end rates with rationale

### Total Cost Estimate
- Engineering-only cost across rate scenarios
- Full team cost across company stages with role breakdown

### Grand Total Summary
- Combined table: calendar time, total hours, total cost across company stages

### Assumptions
- List all assumptions including what is and isn't included

## Step 7: Calculate AI-Assisted Development ROI

Estimate the value produced per hour of AI-assisted development. This answers: **"What did each hour of AI coding time produce?"**

### 7a: Determine AI Active Time

**Method 1: Git History (preferred)**

Run `git log --format="%ai" | sort` to get all commit timestamps. Then:
1. **First commit** = project start
2. **Last commit** = current state
3. **Cluster commits into sessions**: group commits within 4-hour windows as one session
4. **Estimate session duration** from commit density:
   - 1-2 commits in a window: ~1 hour session
   - 3-5 commits: ~2 hour session
   - 6-10 commits: ~3 hour session
   - 10+ commits: ~4 hour session

**Method 2: Fallback Estimate**

If no reliable timestamps, estimate from lines of code:
- Assume AI writes 200-500 lines of meaningful code per hour
- AI active hours = Total LOC / 350

### 7b: Calculate Value per AI Hour

```
Value per AI Hour = Total Code Value (from Step 5) / Estimated AI Active Hours
```

### 7c: AI Efficiency vs. Human Developer

**Speed Multiplier**:
```
Speed Multiplier = Human Dev Hours / AI Active Hours
```

**Cost Efficiency**:
```
Human Cost = Human Hours x Average Rate
AI Cost = Subscription + API costs (estimate from project size)
Savings = Human Cost - AI Cost
ROI = Savings / AI Cost
```

### 7d: Output

Include in the final report:
- Project timeline (first commit to latest)
- Estimated AI active hours and method used
- Value per AI hour table (engineering only and full team equivalent)
- Speed multiplier vs. human developer
- Cost comparison and ROI

---

## Notes

Present the estimate in a clear, professional format suitable for sharing with stakeholders. Include confidence intervals and key assumptions. Highlight areas of highest complexity that drive cost.
