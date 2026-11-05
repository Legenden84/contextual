// Sample project data for the mockup: a .NET clean-architecture solution, Inventory.Api.
// Taken from the design system (docs/design/contextual/components/bundle.js).

import { dedent } from '@/lib/format'

export type ChangeStatus = 'A' | 'M' | 'D'

export interface TreeNode {
  name: string
  path: string
  kind: 'dir' | 'file'
  tokens?: number
  children?: TreeNode[]
  /** Exclusion reason (`ignored: node_modules`, `lock file`, `binary`, `.gitignore`). */
  excluded?: string
  /** Changed on disk since it was loaded into context. */
  changedOnDisk?: boolean
  /** Last modified time from the scan, for real folders. */
  mtimeMs?: number
}

export interface Rule {
  path: string
  type: 'Rule' | 'Skill'
  tokens: number
  content: string
}

export interface Deployment {
  name: string
  limit: number
  /** USD per 1M input tokens. */
  inPrice: number
  /** USD per 1M output tokens. */
  outPrice: number
}

export interface Change {
  path: string
  status: ChangeStatus
  old: string
  new: string
  /** Set when the file changed on disk after it was sent to the model. */
  conflict?: string
}

export const project = {
  name: 'Inventory.Api',
  path: 'C:\\src\\inventory-api',
  branch: 'feature/order-cancel',
}

// ------------------------------------------------------------------ file contents

export const orderOld = dedent(`
  using Inventory.Domain.Common;

  namespace Inventory.Domain.Orders;

  /// <summary>
  /// Aggregate root for a customer order. All state changes go through methods on this type.
  /// </summary>
  public sealed class Order : Entity<Guid>
  {
      private readonly List<OrderLine> _lines = [];

      private Order(Guid id, Guid customerId, DateTimeOffset placedAt) : base(id)
      {
          CustomerId = customerId;
          PlacedAt = placedAt;
          Status = OrderStatus.Pending;
      }

      public Guid CustomerId { get; }
      public DateTimeOffset PlacedAt { get; }
      public DateTimeOffset? ShippedAt { get; private set; }
      public OrderStatus Status { get; private set; }
      public IReadOnlyList<OrderLine> Lines => _lines;

      public decimal Total => _lines.Sum(line => line.UnitPrice * line.Quantity);

      public static Order Place(Guid customerId, DateTimeOffset placedAt) =>
          new(Guid.NewGuid(), customerId, placedAt);

      public void AddLine(Guid productId, int quantity, decimal unitPrice)
      {
          if (Status != OrderStatus.Pending)
              throw new DomainException("Lines can only be added to pending orders.");

          if (quantity <= 0)
              throw new DomainException("Quantity must be positive.");

          _lines.Add(new OrderLine(productId, quantity, unitPrice));
      }

      public void MarkShipped(DateTimeOffset shippedAt)
      {
          Status = OrderStatus.Shipped;
          ShippedAt = shippedAt;
      }
  }`)

export const orderNew = dedent(`
  using Inventory.Domain.Common;

  namespace Inventory.Domain.Orders;

  /// <summary>
  /// Aggregate root for a customer order. All state changes go through methods on this type.
  /// </summary>
  public sealed class Order : Entity<Guid>
  {
      private readonly List<OrderLine> _lines = [];

      private Order(Guid id, Guid customerId, DateTimeOffset placedAt) : base(id)
      {
          CustomerId = customerId;
          PlacedAt = placedAt;
          Status = OrderStatus.Pending;
      }

      public Guid CustomerId { get; }
      public DateTimeOffset PlacedAt { get; }
      public DateTimeOffset? ShippedAt { get; private set; }
      public DateTimeOffset? CancelledAt { get; private set; }
      public string? CancellationReason { get; private set; }
      public OrderStatus Status { get; private set; }
      public IReadOnlyList<OrderLine> Lines => _lines;

      public decimal Total => _lines.Sum(line => line.UnitPrice * line.Quantity);

      public static Order Place(Guid customerId, DateTimeOffset placedAt) =>
          new(Guid.NewGuid(), customerId, placedAt);

      public void AddLine(Guid productId, int quantity, decimal unitPrice)
      {
          if (Status is not OrderStatus.Pending)
              throw new DomainException("Lines can only be added to pending orders.");

          if (quantity <= 0)
              throw new DomainException("Quantity must be positive.");

          _lines.Add(new OrderLine(productId, quantity, unitPrice));
      }

      public void MarkShipped(DateTimeOffset shippedAt)
      {
          if (Status is not OrderStatus.Pending)
              throw new DomainException($"Order {Id} cannot be shipped from status {Status}.");

          Status = OrderStatus.Shipped;
          ShippedAt = shippedAt;
      }

      /// <summary>
      /// Cancels the order. Cancelling an already cancelled order is a no-op.
      /// </summary>
      /// <exception cref="DomainException">The order has already shipped.</exception>
      public void Cancel(string reason, DateTimeOffset cancelledAt)
      {
          ArgumentException.ThrowIfNullOrWhiteSpace(reason);

          if (Status is OrderStatus.Shipped)
              throw new DomainException($"Order {Id} has shipped and cannot be cancelled.");

          if (Status is OrderStatus.Cancelled)
              return;

          Status = OrderStatus.Cancelled;
          CancelledAt = cancelledAt;
          CancellationReason = reason;
      }
  }`)

export const cancelCommand = dedent(`
  using MediatR;

  namespace Inventory.Application.Orders.Commands.CancelOrder;

  /// <summary>
  /// Cancels a pending order.
  /// </summary>
  /// <param name="OrderId">The order to cancel.</param>
  /// <param name="Reason">Free-text reason, shown to support staff.</param>
  /// <returns><c>true</c> if the order was cancelled; <c>false</c> if it does not exist.</returns>
  public sealed record CancelOrderCommand(Guid OrderId, string Reason) : IRequest<bool>;`)

export const cancelHandler = dedent(`
  using Inventory.Application.Abstractions;
  using MediatR;

  namespace Inventory.Application.Orders.Commands.CancelOrder;

  /// <summary>
  /// Handles <see cref="CancelOrderCommand"/> by loading the aggregate and calling
  /// <see cref="Domain.Orders.Order.Cancel"/>. Domain rules stay in the domain.
  /// </summary>
  internal sealed class CancelOrderHandler(
      IOrderRepository orders,
      IUnitOfWork unitOfWork,
      TimeProvider clock) : IRequestHandler<CancelOrderCommand, bool>
  {
      public async Task<bool> Handle(CancelOrderCommand request, CancellationToken cancellationToken)
      {
          var order = await orders.GetByIdAsync(request.OrderId, cancellationToken);
          if (order is null)
          {
              return false;
          }

          order.Cancel(request.Reason, clock.GetUtcNow());
          await unitOfWork.SaveChangesAsync(cancellationToken);

          return true;
      }
  }`)

export const endpointsOld = dedent(`
  using Inventory.Application.Orders.Commands.CreateOrder;
  using Inventory.Application.Orders.Queries.GetOrderById;
  using MediatR;

  namespace Inventory.Api.Endpoints;

  public static class OrderEndpoints
  {
      public static IEndpointRouteBuilder MapOrderEndpoints(this IEndpointRouteBuilder app)
      {
          var group = app.MapGroup("/orders").WithTags("Orders");

          group.MapGet("/{id:guid}", async (Guid id, ISender sender, CancellationToken ct) =>
          {
              var order = await sender.Send(new GetOrderByIdQuery(id), ct);
              return order is null ? Results.NotFound() : Results.Ok(order);
          })
          .WithName("GetOrderById");

          group.MapPost("/", async (CreateOrderCommand command, ISender sender, CancellationToken ct) =>
          {
              var id = await sender.Send(command, ct);
              return Results.CreatedAtRoute("GetOrderById", new { id }, new { id });
          });

          return app;
      }
  }`)

export const endpointsNew = dedent(`
  using Inventory.Application.Orders.Commands.CancelOrder;
  using Inventory.Application.Orders.Commands.CreateOrder;
  using Inventory.Application.Orders.Queries.GetOrderById;
  using MediatR;
  using Microsoft.AspNetCore.Mvc;

  namespace Inventory.Api.Endpoints;

  public static class OrderEndpoints
  {
      public static IEndpointRouteBuilder MapOrderEndpoints(this IEndpointRouteBuilder app)
      {
          var group = app.MapGroup("/orders").WithTags("Orders");

          group.MapGet("/{id:guid}", async (Guid id, ISender sender, CancellationToken ct) =>
          {
              var order = await sender.Send(new GetOrderByIdQuery(id), ct);
              return order is null ? Results.NotFound() : Results.Ok(order);
          })
          .WithName("GetOrderById");

          group.MapPost("/", async (CreateOrderCommand command, ISender sender, CancellationToken ct) =>
          {
              var id = await sender.Send(command, ct);
              return Results.CreatedAtRoute("GetOrderById", new { id }, new { id });
          });

          group.MapPost("/{id:guid}/cancel", async (Guid id, [FromQuery] string reason, ISender sender, CancellationToken ct) =>
          {
              var cancelled = await sender.Send(new CancelOrderCommand(id, reason), ct);
              return cancelled ? Results.NoContent() : Results.NotFound();
          })
          .WithName("CancelOrder")
          .ProducesProblem(StatusCodes.Status409Conflict);

          return app;
      }
  }`)

export const legacyOld = dedent(`
  using Inventory.Application.Abstractions;
  using Inventory.Domain.Orders;

  namespace Inventory.Application.Orders.Commands.LegacyCancel;

  [Obsolete("Use CancelOrderCommand instead.")]
  public sealed class LegacyCancelHandler(IOrderRepository orders, IUnitOfWork unitOfWork)
  {
      public async Task CancelAsync(Guid orderId, CancellationToken cancellationToken = default)
      {
          var order = await orders.GetByIdAsync(orderId, cancellationToken)
              ?? throw new InvalidOperationException($"Order {orderId} not found.");

          // TODO: bypasses the aggregate and ignores shipped orders.
          typeof(Order).GetProperty(nameof(Order.Status))!
              .SetValue(order, OrderStatus.Cancelled);

          await unitOfWork.SaveChangesAsync(cancellationToken);
      }
  }`)

export const appsettings = dedent(`
  {
    "ConnectionStrings": {
      "Inventory": "Server=localhost,1433;Database=Inventory;User Id=sa;TrustServerCertificate=True"
    },
    "Logging": {
      "LogLevel": {
        "Default": "Information",
        "Microsoft.AspNetCore": "Warning"
      }
    },
    "Orders": {
      "CancellationWindowHours": 24,
      "AllowCancelAfterPicking": false
    },
    "AllowedHosts": "*"
  }`)

export const applicationCsproj = dedent(`
  <Project Sdk="Microsoft.NET.Sdk">

    <PropertyGroup>
      <TargetFramework>net8.0</TargetFramework>
      <Nullable>enable</Nullable>
      <ImplicitUsings>enable</ImplicitUsings>
      <RootNamespace>Inventory.Application</RootNamespace>
    </PropertyGroup>

    <ItemGroup>
      <PackageReference Include="MediatR" Version="12.4.1" />
      <PackageReference Include="FluentValidation" Version="11.10.0" />
    </ItemGroup>

    <ItemGroup>
      <ProjectReference Include="..\\Inventory.Domain\\Inventory.Domain.csproj" />
    </ItemGroup>

  </Project>`)

export const agentsMd = dedent(`
  # AGENTS.md — Inventory.Api

  ## Architecture
  - Clean architecture: Domain → Application → Infrastructure → Api.
  - Domain has no dependencies. Business rules live on aggregates, not in handlers.
  - One type per file. File name matches the type name.

  ## Conventions
  - Commands and queries are MediatR \`IRequest<T>\` records in \`Application/<Feature>/Commands|Queries/<Name>/\`.
  - Handlers are \`internal sealed\` and use primary constructors.
  - Use \`TimeProvider\`, never \`DateTime.UtcNow\`.
  - Public members get XML doc comments.`)

const copilotInstructions = dedent(`
  # Copilot instructions

  - Target .NET 8 and C# 12. Prefer primary constructors and collection expressions.
  - Minimal APIs live in \`src/Inventory.Api/Endpoints\`, one static class per aggregate.
  - Return \`Results.Problem\` for domain errors; never throw from an endpoint.
  - Tests use xUnit and FluentAssertions; name them \`Method_State_Expected\`.`)

const csharpStyle = dedent(`
  ---
  description: C# style for Inventory.Api
  globs: "**/*.cs"
  ---
  - File-scoped namespaces.
  - \`var\` when the type is obvious from the right-hand side.
  - Pattern matching (\`is not\`, \`is null\`) over \`!=\` and \`== null\`.
  - Async methods end in \`Async\` and take a \`CancellationToken\` last.`)

const claudeMd = dedent(`
  # CLAUDE.md

  Build with \`dotnet build Inventory.sln\`; test with \`dotnet test\`.
  EF Core migrations live in \`src/Inventory.Infrastructure/Migrations\`.
  Don't edit \`*.Designer.cs\` or the model snapshot by hand.`)

const efSkill = dedent(`
  ---
  name: ef-migrations
  description: Add or update an EF Core migration for Inventory.Infrastructure.
  ---
  # EF migrations

  1. Change the entity configuration in \`Persistence/Configurations\`.
  2. Run \`dotnet ef migrations add <Name> -p src/Inventory.Infrastructure -s src/Inventory.Api\`.
  3. Review the generated migration; never edit the snapshot.`)

export const outputPrompt = dedent(`
  You are helping edit files in a local project. The user reviews every change
  and applies it manually. You cannot write to disk.

  ## Output format
  - Explain the change in a few sentences first.
  - For every file you add or change, return its COMPLETE new contents.
    Never return partial snippets, "..." or "unchanged" placeholders.
  - Put each file in its own fenced block, directly after a header line:

  ### FILE: src/Path/Relative/To/Root.cs
  \`\`\`csharp
  <full file contents>
  \`\`\`

  - To delete a file, write a single line: ### DELETE: src/Path/To/File.cs
  - Only touch files that are included in the context.
  - Follow the project rules provided above this prompt.`)

/** Contents available for the read-only preview, by project path. */
export const fileContents: Record<string, string> = {
  'src/Inventory.Domain/Orders/Order.cs': orderOld,
  'src/Inventory.Api/Endpoints/OrderEndpoints.cs': endpointsOld,
  'src/Inventory.Application/Orders/Commands/LegacyCancel/LegacyCancelHandler.cs': legacyOld,
  'src/Inventory.Api/appsettings.json': appsettings,
  'src/Inventory.Application/Inventory.Application.csproj': applicationCsproj,
  'AGENTS.md': agentsMd,
  '.github/copilot-instructions.md': copilotInstructions,
  '.claude/skills/ef-migrations/SKILL.md': efSkill,
}

// ------------------------------------------------------------------ file tree

type Spec = [name: string, tokensOrChildren: number | Spec[] | null, flags?: { x?: string; d?: boolean }]

function build(specs: Spec[], parent: string): TreeNode[] {
  return specs.map(([name, v, flags = {}]) => {
    const path = parent + name
    const isDir = Array.isArray(v) || (v === null && !/\.\w+$/.test(name))
    return {
      name,
      path,
      kind: isDir ? 'dir' : 'file',
      tokens: typeof v === 'number' ? v : undefined,
      children: Array.isArray(v) ? build(v, path + '/') : isDir ? [] : undefined,
      excluded: flags.x,
      changedOnDisk: flags.d,
    }
  })
}

export const tree: TreeNode[] = build(
  [
    ['.claude', [['skills', [['ef-migrations', [['SKILL.md', 610]]]]]]],
    ['.github', [['copilot-instructions.md', 420]]],
    [
      'src',
      [
        [
          'Inventory.Api',
          [
            ['bin', null, { x: 'ignored: bin' }],
            ['obj', null, { x: 'ignored: obj' }],
            [
              'Endpoints',
              [
                ['OrderEndpoints.cs', 640, { d: true }],
                ['ProductEndpoints.cs', 720],
              ],
            ],
            ['appsettings.Development.json', 140],
            ['appsettings.json', 210],
            ['Inventory.Api.csproj', 260],
            ['Program.cs', 980],
          ],
        ],
        [
          'Inventory.Application',
          [
            [
              'Abstractions',
              [
                ['IOrderRepository.cs', 230],
                ['IUnitOfWork.cs', 120],
              ],
            ],
            [
              'Orders',
              [
                [
                  'Commands',
                  [
                    [
                      'CreateOrder',
                      [
                        ['CreateOrderCommand.cs', 380],
                        ['CreateOrderHandler.cs', 1120],
                        ['CreateOrderValidator.cs', 540],
                      ],
                    ],
                    ['LegacyCancel', [['LegacyCancelHandler.cs', 410]]],
                  ],
                ],
                [
                  'Queries',
                  [
                    [
                      'GetOrderById',
                      [
                        ['GetOrderByIdQuery.cs', 160],
                        ['GetOrderByIdHandler.cs', 690],
                        ['OrderDto.cs', 310],
                      ],
                    ],
                  ],
                ],
              ],
            ],
            ['DependencyInjection.cs', 520],
            ['Inventory.Application.csproj', 240],
          ],
        ],
        [
          'Inventory.Domain',
          [
            [
              'Common',
              [
                ['DomainException.cs', 150],
                ['Entity.cs', 340],
              ],
            ],
            [
              'Orders',
              [
                ['Order.cs', 1460],
                ['OrderLine.cs', 420],
                ['OrderStatus.cs', 90],
              ],
            ],
          ],
        ],
        [
          'Inventory.Infrastructure',
          [
            [
              'Persistence',
              [
                ['InventoryDbContext.cs', 860],
                ['OrderRepository.cs', 740],
              ],
            ],
            [
              'Migrations',
              [
                ['20261028_AddOrderStatus.cs', 2400],
                ['20261028_AddOrderStatus.Designer.cs', 16800],
                ['InventoryDbContextModelSnapshot.cs', 16900],
              ],
            ],
          ],
        ],
      ],
    ],
    [
      'tests',
      [
        [
          'Inventory.Domain.Tests',
          [
            ['OrderTests.cs', 1680],
            ['bin', null, { x: 'ignored: bin' }],
          ],
        ],
      ],
    ],
    ['node_modules', null, { x: 'ignored: node_modules' }],
    ['.gitignore', 80],
    ['AGENTS.md', 560],
    ['Inventory.sln', 1900],
    ['Inventory.sln.DotSettings.user', 300, { x: '.gitignore' }],
    ['docs-architecture.png', null, { x: 'binary' }],
    ['packages.lock.json', 48200, { x: 'lock file' }],
  ],
  '',
)

/** Files ticked when the project opens (12 files, ~7.5k tokens). */
export const initialSelection = [
  'src/Inventory.Api/Endpoints/OrderEndpoints.cs',
  'src/Inventory.Api/appsettings.json',
  'src/Inventory.Api/Program.cs',
  'src/Inventory.Application/Abstractions/IOrderRepository.cs',
  'src/Inventory.Application/Abstractions/IUnitOfWork.cs',
  'src/Inventory.Application/Orders/Commands/CreateOrder/CreateOrderCommand.cs',
  'src/Inventory.Application/Orders/Commands/CreateOrder/CreateOrderHandler.cs',
  'src/Inventory.Application/Orders/Commands/LegacyCancel/LegacyCancelHandler.cs',
  'src/Inventory.Domain/Common/DomainException.cs',
  'src/Inventory.Domain/Orders/Order.cs',
  'src/Inventory.Domain/Orders/OrderStatus.cs',
  'tests/Inventory.Domain.Tests/OrderTests.cs',
]

/** Folders expanded when the project opens. */
export const initialOpen = [
  '.github',
  'src',
  'src/Inventory.Api',
  'src/Inventory.Api/Endpoints',
  'src/Inventory.Application',
  'src/Inventory.Application/Abstractions',
  'src/Inventory.Application/Orders',
  'src/Inventory.Application/Orders/Commands',
  'src/Inventory.Application/Orders/Commands/LegacyCancel',
  'src/Inventory.Domain',
  'src/Inventory.Domain/Common',
  'src/Inventory.Domain/Orders',
  'tests',
  'tests/Inventory.Domain.Tests',
]

// ------------------------------------------------------------------ rules, deployments

export const rules: Rule[] = [
  { path: 'AGENTS.md', type: 'Rule', tokens: 560, content: agentsMd },
  { path: '.github/copilot-instructions.md', type: 'Rule', tokens: 420, content: copilotInstructions },
  { path: '.cursor/rules/csharp-style.mdc', type: 'Rule', tokens: 380, content: csharpStyle },
  { path: 'CLAUDE.md', type: 'Rule', tokens: 290, content: claudeMd },
  { path: '.claude/skills/ef-migrations/SKILL.md', type: 'Skill', tokens: 610, content: efSkill },
]

export const initialRulesOn = ['AGENTS.md', '.github/copilot-instructions.md', '.cursor/rules/csharp-style.mdc']

export const deployments: Deployment[] = [
  { name: 'gpt-4.1', limit: 128000, inPrice: 2.0, outPrice: 8.0 },
  { name: 'DeepSeek-V3', limit: 64000, inPrice: 1.14, outPrice: 4.56 },
  { name: 'gpt-4.1-mini', limit: 128000, inPrice: 0.4, outPrice: 1.6 },
]

export const foundry = {
  endpoint: 'https://tl-foundry-dev.services.ai.azure.com/models',
  apiKey: 'sk-foundry-dev-7c1e0b5d2a9f48e6b3f9a',
  apiVersion: '2024-05-01-preview',
}

export const defaultExclusions = [
  '**/node_modules/**',
  '**/.venv/**',
  '**/venv/**',
  '**/__pycache__/**',
  '**/bin/**',
  '**/obj/**',
  '**/dist/**',
  '**/build/**',
  '**/.git/**',
  'package-lock.json',
  'yarn.lock',
  'pnpm-lock.yaml',
  'poetry.lock',
  'Pipfile.lock',
  'packages.lock.json',
  'Cargo.lock',
]

export const recentProjects = [
  { name: 'Inventory.Api', path: 'C:\\src\\inventory-api', when: '2 hours ago', stack: '.NET' },
  { name: 'Catalog.Web', path: 'C:\\src\\catalog-web', when: 'Yesterday', stack: 'TypeScript' },
  { name: 'scheduler-worker', path: 'D:\\work\\scheduler-worker', when: '3 days ago', stack: 'Python' },
  { name: 'Billing.Service', path: 'C:\\src\\billing-service', when: 'Oct 28', stack: '.NET' },
  { name: 'infra-bicep', path: 'C:\\src\\infra-bicep', when: 'Oct 12', stack: 'Bicep' },
]

// ------------------------------------------------------------------ the conversation

export const changes: Change[] = [
  { path: 'src/Inventory.Domain/Orders/Order.cs', status: 'M', old: orderOld, new: orderNew },
  {
    path: 'src/Inventory.Application/Orders/Commands/CancelOrder/CancelOrderCommand.cs',
    status: 'A',
    old: '',
    new: cancelCommand,
  },
  {
    path: 'src/Inventory.Application/Orders/Commands/CancelOrder/CancelOrderHandler.cs',
    status: 'A',
    old: '',
    new: cancelHandler,
  },
  {
    path: 'src/Inventory.Api/Endpoints/OrderEndpoints.cs',
    status: 'M',
    old: endpointsOld,
    new: endpointsNew,
    conflict: 'Changed 4 min ago (2 lines). The proposal was made against the older version.',
  },
  {
    path: 'src/Inventory.Application/Orders/Commands/LegacyCancel/LegacyCancelHandler.cs',
    status: 'D',
    old: legacyOld,
    new: '',
  },
]

export const userPrompt =
  'Add order cancellation. `Order.Cancel()` must throw if the order has shipped, and `MarkShipped` should only work from Pending. Add a `CancelOrderCommand` with a handler and a `POST /orders/{id}/cancel` endpoint. Remove `LegacyCancelHandler`.'

export const assistantResponse = [
  'Cancellation rules now live on the aggregate. `Order.Cancel(reason, cancelledAt)` throws a `DomainException` for shipped orders and is a no-op when already cancelled. The handler only loads, calls and saves.',
  '',
  '- [M] `Order.cs` — adds `Cancel`, `CancelledAt`, `CancellationReason`; guards `MarkShipped`.',
  '- [A] `CancelOrderCommand.cs`, `CancelOrderHandler.cs` — one type per file, as AGENTS.md asks.',
  '- [M] `OrderEndpoints.cs` — maps `POST /orders/{id:guid}/cancel`, 204 / 404 / 409.',
  '- [D] `LegacyCancelHandler.cs` — no remaining references.',
  '',
  '### FILE: src/Inventory.Application/Orders/Commands/CancelOrder/CancelOrderHandler.cs',
  '```csharp',
  cancelHandler,
  '```',
  '',
  '### FILE: src/Inventory.Domain/Orders/Order.cs',
  '```csharp',
  orderNew,
  '```',
  '',
  'Run `dotnet test tests/Inventory.Domain.Tests` after applying — `OrderTests.MarkShipped_FromCancelled_Throws` will need a new case.',
].join('\n')

/** Fixed token costs on top of files and rules (system prompt, the user's message, the reply in history). */
export const PROMPT_TOKENS = 168
export const USER_MESSAGE_TOKENS = 96
export const REPLY_TOKENS = 2140
/** Output tokens assumed for the per-message cost estimate. */
export const EST_OUTPUT_TOKENS = 1800
