import argparse
import asyncio
import json
import sys
from .daemon import Core, directories, encode


async def control(action, payload):
    runtime = directories()[2]
    reader, writer = await asyncio.open_unix_connection(str(runtime / 'core.sock'), limit=1048576)
    try:
        writer.write(encode(dict(schemaVersion=1, requestId=1, action=action, payload=payload)))
        await writer.drain()
        while line := await asyncio.wait_for(reader.readline(), 10):
            result = json.loads(line)
            if result.get('requestId') == 1:
                print(json.dumps(result, ensure_ascii=False))
                return 0 if result['ok'] else 1
        return 1
    finally:
        writer.close()
        await writer.wait_closed()


def main():
    parser = argparse.ArgumentParser(description='Управление Drift Shell')
    parser.add_argument('action', nargs='?', default='daemon')
    parser.add_argument('payload', nargs='?', default='{}')
    parser.add_argument('--safe', action='store_true')
    args = parser.parse_args()
    try:
        if args.action == 'daemon':
            asyncio.run(Core(safe=args.safe).run())
            return 0
        payload = json.loads(args.payload)
        if args.action == 'zone' and isinstance(payload, int):
            payload = {'slot': payload}
        return asyncio.run(control(args.action, payload))
    except KeyboardInterrupt:
        return 0
    except (OSError, RuntimeError, ValueError, asyncio.TimeoutError) as error:
        print('Drift Shell: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
