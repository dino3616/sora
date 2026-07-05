//************************************************************************************************
// Sora Bridge — Studio One 内で Sora の編集リクエストを実行する拡張
//
// プロトコル(実機検証済み・2026-07-05):
//   1. Sora が JSON リクエストを $USERCONTENT/SoraBridge/inbox/ へ書く
//   2. EditTask「Apply Sora Bridge Inbox」(または Sora Surface のトリガー)が実行される
//   3. 本スクリプトが inbox を読み、interpretCommand で Studio One 内部コマンドを実行
//   4. 処理済み JSON を outbox へ移動する
//
// リクエスト形式:
//   { "type": "command", "category": "...", "name": "...", "args": {...} }
//   { "type": "import_file", "path": "/абс/path/to/file.mid" }
//   { "type": "dump_command", "category": "...", "name": "..." }
//
// 設計方針: GUI アラートは出さない(Sora 側から非対話で駆動されるため)。
// 結果は Host.Console と outbox のステータスファイルで報告する。
//************************************************************************************************

function SoraBridgeCore ()
{
	this.getInbox = function ()
	{
		return Host.Url ("local://$USERCONTENT/SoraBridge/inbox", true);
	}

	this.getOutbox = function ()
	{
		return Host.Url ("local://$USERCONTENT/SoraBridge/outbox", true);
	}

	this.completeCommandFile = function (path)
	{
		let outPath = this.getOutbox ();
		outPath.descend (path.name);
		outPath.makeUnique ();
		let file = Host.IO.File (path);
		if(!file.moveTo (outPath))
		{
			if(file.copyTo (outPath))
				file.remove ();
		}
	}

	this.readTextFile = function (path)
	{
		let file = Host.IO.openTextFile (path);
		if(!file)
			return "";

		let lines = [];
		while(!file.endOfStream)
			lines.push (file.readLine ());
		file.close ();
		return lines.join ("\n");
	}

	this.writeOutboxTextFile = function (name, lines)
	{
		let outPath = this.getOutbox ();
		outPath.descend (name);
		outPath.makeUnique ();

		let file = Host.IO.createTextFile (outPath);
		if(!file)
			return false;

		for(let i = 0; i < lines.length; i++)
			file.writeLine (lines[i]);
		file.close ();
		return true;
	}

	this.makeAttributes = function (pairs)
	{
		if(!pairs)
			return null;

		let args = [];
		for(let key in pairs)
		{
			args.push (key);
			args.push (pairs[key]);
		}
		return Host.Attributes (args);
	}

	// inbox の先頭リクエストを 1 件実行する。inbox が空なら何もせず true。
	this.applyNextCommand = function ()
	{
		let iter = Host.IO.findFiles (this.getInbox (), "*.json");
		if(iter.done ())
		{
			Host.Console.writeLine ("SoraBridge: inbox empty");
			return true;
		}

		let path = iter.next ();
		let raw = this.readTextFile (path);
		let request;
		try
		{
			request = JSON.parse (raw);
		}
		catch(e)
		{
			Host.Console.writeLine ("SoraBridge: invalid JSON in " + path.name + ": " + e);
			this.writeOutboxTextFile ("error-" + path.name + ".txt", ["invalid JSON: " + e]);
			this.completeCommandFile (path);
			return false;
		}

		let ok = false;

		if(request.type == "command")
		{
			let attrs = this.makeAttributes (request.args);
			ok = Host.GUI.Commands.interpretCommand (request.category, request.name, false, attrs);
			Host.Console.writeLine ("SoraBridge: command " + request.category + "/" + request.name + " -> " + ok);
			if(ok)
				this.completeCommandFile (path);
			return ok;
		}

		if(request.type == "import_file")
		{
			let attrs = this.makeAttributes ({ File: request.path });
			ok = Host.GUI.Commands.interpretCommand ("Song", "Import File", false, attrs);
			Host.Console.writeLine ("SoraBridge: import_file " + request.path + " -> " + ok);
			if(ok)
				this.completeCommandFile (path);
			return ok;
		}

		if(request.type == "dump_command")
		{
			let command = Host.GUI.Commands.findCommand (request.category, request.name);
			let lines = [];
			lines.push ("category=" + request.category);
			lines.push ("name=" + request.name);
			lines.push ("command=" + command);
			if(command)
			{
				for(let key in command)
				{
					try
					{
						lines.push (key + "=" + command[key]);
					}
					catch(e)
					{
						lines.push (key + "=<error>");
					}
				}
			}
			this.writeOutboxTextFile ("dump-command-" + request.category + "-" + request.name + ".txt", lines);
			this.completeCommandFile (path);
			return true;
		}

		Host.Console.writeLine ("SoraBridge: unsupported request type " + request.type);
		this.writeOutboxTextFile ("error-" + path.name + ".txt", ["unsupported request type: " + request.type]);
		this.completeCommandFile (path);
		return false;
	}
}

//************************************************************************************************
// ProgramService: 「Sora / Apply Next Command」コマンドを登録する
// (Sora Surface からの interpretCommand 経路。§11.2.1)
//************************************************************************************************

function SoraBridgeService ()
{
	this.interfaces = [Host.Interfaces.IComponent, Host.Interfaces.ICommandHandler];
	this.category = "Sora";
	this.commandName = "Apply Next Command";

	this.initialize = function ()
	{
		Host.Console.writeLine ("SoraBridge: service initialize");
		Host.GUI.Commands.registerCommand (this.category, this.commandName, "Sora", this.commandName);
		Host.GUI.Commands.addHandler (this);
		return Host.Results.kResultOk;
	}

	this.terminate = function ()
	{
		Host.GUI.Commands.removeHandler (this);
		Host.GUI.Commands.unregisterCommand (this.category, this.commandName);
		Host.Console.writeLine ("SoraBridge: service terminate");
		return Host.Results.kResultOk;
	}

	this.interpretCommand = function (msg)
	{
		if(msg.category != this.category || msg.name != this.commandName)
			return false;

		if(msg.checkOnly)
			return true;

		let core = new SoraBridgeCore ();
		return core.applyNextCommand ();
	}

	this.checkCommandCategory = function (category)
	{
		return category == this.category;
	}
}

function createInstance (args)
{
	return new SoraBridgeService ();
}

//************************************************************************************************
// EditTask: メニュー「トラック > Apply Sora Bridge Inbox」(実機検証済みの手動経路)
//************************************************************************************************

function SoraBridgeApplyTask ()
{
	this.interfaces = [Host.Interfaces.IEditTask];

	this.prepareEdit = function (context)
	{
		return Host.Results.kResultOk;
	}

	this.performEdit = function (context)
	{
		let core = new SoraBridgeCore ();
		let ok = core.applyNextCommand ();
		return ok ? Host.Results.kResultOk : Host.Results.kResultFailed;
	}
}

function createApplyTask (args)
{
	return new SoraBridgeApplyTask ();
}
